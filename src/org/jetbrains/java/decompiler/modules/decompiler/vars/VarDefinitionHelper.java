// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.VarNamesCollector;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.ExprProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.StackVarsProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.sforms.SSAUConstructorSparseEx;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarTypeProcessor.FinalType;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructLocalVariableTableAttribute.LocalVariable;
import org.jetbrains.java.decompiler.struct.attr.StructMethodParametersAttribute;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.TypeFamily;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.struct.gen.generics.GenericType;
import org.jetbrains.java.decompiler.util.ArrayHelper;
import org.jetbrains.java.decompiler.util.InterpreterUtil;
import org.jetbrains.java.decompiler.util.Pair;
import org.jetbrains.java.decompiler.util.StatementIterator;

import java.util.*;
import java.util.Map.Entry;

public class VarDefinitionHelper {

  private final HashMap<Integer, Statement> mapVarDefStatements;

  // statement.id, defined vars
  private final HashMap<Integer, HashSet<Integer>> mapStatementVars;

  private final HashSet<Integer> implDefVars;

  private final VarProcessor varproc;

  private final RootStatement root;
  private final StructMethod mt;
  private final Set<Integer> semanticParameterSlots;
  private final Map<VarVersionPair, String> clashingNames = new HashMap<>();
  private final boolean j2meStrictSlotMerge;
  private final Map<VarVersionPair, Set<VarType>> legacySlotTypeEvidence;
  private final Map<VarVersionPair, Set<VarType>> assignmentUseUpperBounds = new HashMap<>();
  private final Set<VarVersionPair> nullAssignmentDefinitions = new HashSet<>();

  public VarDefinitionHelper(RootStatement root, StructMethod mt, VarProcessor varproc) {
    mapVarDefStatements = new HashMap<>();
    mapStatementVars = new HashMap<>();
    implDefVars = new HashSet<>();

    this.varproc = varproc;
    this.root = root;
    this.mt = mt;
    this.semanticParameterSlots = findSemanticParameterSlots();
    this.j2meStrictSlotMerge = DecompilerContext.getOption(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE) || mt.hasAttribute(StructGeneralAttribute.ATTRIBUTE_STACK_MAP);
    this.legacySlotTypeEvidence = j2meStrictSlotMerge ? collectLegacySlotTypeEvidence() : new HashMap<>();

    VarNamesCollector vc = varproc.getVarNamesCollector();

    boolean thisvar = !mt.hasModifier(CodeConstants.ACC_STATIC);

    MethodDescriptor md = MethodDescriptor.parseDescriptor(mt.getDescriptor());

    int paramcount = 0;
    if (thisvar) {
      paramcount = 1;
    }
    paramcount += md.params.length;

    List<StructMethodParametersAttribute.Entry> methodParameters = null;
    if (DecompilerContext.getOption(IFernflowerPreferences.USE_METHOD_PARAMETERS)) {
      StructMethodParametersAttribute attr = mt.getAttribute(StructGeneralAttribute.ATTRIBUTE_METHOD_PARAMETERS);
      if (attr != null) {
        methodParameters = attr.getEntries();
      }
    }

    // method parameters are implicitly defined
    int varindex = 0;
    int paramIndex = 0;
    for (int i = 0; i < paramcount; i++) {
      implDefVars.add(varindex);
      VarVersionPair vpp = new VarVersionPair(varindex, 0);
      varproc.markParam(vpp);
      if (varindex != 0 || !thisvar) {
        if (methodParameters != null && paramIndex < methodParameters.size()) {
          varproc.setVarName(vpp, vc.getFreeName(methodParameters.get(paramIndex).myName));
          paramIndex++;
        } else {
          varproc.setVarName(vpp, vc.getFreeName(varindex));
        }
      }

      if (thisvar) {
        if (i == 0) {
          varindex++;
        }
        else {
          varindex += md.params[i - 1].stackSize;
        }
      }
      else {
        varindex += md.params[i].stackSize;
      }
    }

    if (thisvar) {
      StructClass current_class = (StructClass)DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS);

      varproc.getThisVars().put(new VarVersionPair(0, 0), current_class.qualifiedName);
      varproc.setVarName(new VarVersionPair(0, 0), "this");
      vc.addName("this");
    }

    // Strict J2ME mode must still run the final merge pass: canMergeTypes guards
    // each merge with legacy StackMap evidence, while skipping the pass leaves
    // valid same-slot SSA splits declared without an initializer.
    mergeVars(root);

    // catch variables are implicitly defined
    Deque<Statement> stack = new ArrayDeque<>();
    stack.add(root);

    while (!stack.isEmpty()) {
      Statement st = stack.removeFirst();

      List<VarExprent> lstVars = st.getImplicitlyDefinedVars();

      if (lstVars != null) {
        for (VarExprent var : lstVars) {
          implDefVars.add(var.getIndex());
          varproc.setVarName(new VarVersionPair(var), vc.getFreeName(var.getIndex()));
          var.setDefinition(true);
        }
      }

      stack.addAll(st.getStats());
    }

    initStatement(root);

    ValidationHelper.validateVars(root, var -> var.getVarType() != VarType.VARTYPE_UNKNOWN, "Var type not set!");
  }

  public void setVarDefinitions() {
    VarNamesCollector vc = varproc.getVarNamesCollector();

    for (Entry<Integer, Statement> en : mapVarDefStatements.entrySet()) {
      Statement stat = en.getValue();
      int index = en.getKey();

      if (implDefVars.contains(index)) {
        // already implicitly defined
        continue;
      }

      varproc.setVarName(new VarVersionPair(index, 0), vc.getFreeName(index));

      // special case for
      if (stat instanceof DoStatement) {
        DoStatement dstat = (DoStatement)stat;
        if (dstat.getLooptype() == DoStatement.Type.FOR) {

          if (dstat.getInitExprent() != null && setDefinition(dstat.getInitExprent(), index)) {
            continue;
          }
          else {
            List<Exprent> lstSpecial = Arrays.asList(dstat.getConditionExprent(), dstat.getIncExprent());
            for (VarExprent var : getAllVars(lstSpecial)) {
              if (var.getIndex() == index) {
                stat = stat.getParent();
                break;
              }
            }
          }
        }
        else if (dstat.getLooptype() == DoStatement.Type.FOR_EACH) {
          if (dstat.getInitExprent() != null && dstat.getInitExprent() instanceof VarExprent) {
            VarExprent var = (VarExprent)dstat.getInitExprent();
            if (var.getIndex() == index) {
              var.setDefinition(true);
              continue;
            }
          }
        }
      }

      Statement first = findFirstBlock(stat, index);

      List<Exprent> lst;
      if (first == null) {
        lst = stat.getVarDefinitions();
      } else if (first.getExprents() == null) {
        lst = first.getVarDefinitions();
      } else {
        lst = first.getExprents();
      }

      boolean defset = false;

      // search for the first assignment to var [index]
      int addindex = 0;
      for (Exprent expr : lst) {
        if (setDefinition(expr, index)) {
          defset = true;
          break;
        }
        else {
          boolean foundvar = false;
          for (Exprent exp : expr.getAllExprents(true)) {
            if (exp instanceof VarExprent && ((VarExprent)exp).getIndex() == index) {
              foundvar = true;
              break;
            }
          }
          if (foundvar) {
            break;
          }
        }
        addindex++;
      }

      if (!defset) {
        VarExprent var = new VarExprent(index, varproc.getVarType(new VarVersionPair(index, 0)), varproc);
        var.setDefinition(true);

        LocalVariable lvt = findLVT(index, stat);
        if (lvt != null) {
          var.setLVT(lvt);
        }

        lst.add(addindex, var);
      }
    }

    mergeVars(root);
    propagateLVTs(root);
    setNonFinal(root, new HashSet<>());
    clashingNames.putAll(ClashingNameProcessor.remap(root, mt, varproc));
  }


  // *****************************************************************************
  // private methods
  // *****************************************************************************

  private LocalVariable findLVT(int index, Statement stat) {
    if (stat.getExprents() == null) {
      for (Statement st : stat.getStats()) {
        LocalVariable lvt = findLVT(index, st);
        if (lvt != null) {
          return lvt;
        }
      }

      for (Exprent exp : stat.getStatExprents()) {
        LocalVariable lvt = findLVT(index, exp);
        if (lvt != null) {
          return lvt;
        }
      }
    }
    else {
      for (Exprent exp : stat.getExprents()) {
        LocalVariable lvt = findLVT(index, exp);
        if (lvt != null) {
          return lvt;
        }
      }
    }
    return null;
  }

  private LocalVariable findLVT(int index, Exprent exp) {
    for (Exprent e: exp.getAllExprents(false)) {
      LocalVariable lvt = findLVT(index, e);
      if (lvt != null) {
        return lvt;
      }
    }

    if (!(exp instanceof VarExprent)) {
      return null;
    }

    VarExprent var = (VarExprent)exp;
    return var.getIndex() == index ? var.getLVT() : null;
  }

  private Statement findFirstBlock(Statement stat, int varindex) {

    LinkedList<Statement> stack = new LinkedList<>();
    stack.add(stat);

    while (!stack.isEmpty()) {
      Statement st = stack.remove(0);

      if (stack.isEmpty() || mapStatementVars.get(st.id).contains(varindex)) {

        if (st.isLabeled() && !stack.isEmpty()) {
          return st;
        }

        if (st.getExprents() != null) {
          return st;
        }
        else {
          stack.clear();

          switch (st.type) {
            case SEQUENCE:
              stack.addAll(0, st.getStats());
              break;
            case IF:
            case ROOT:
            case SWITCH:
            case SYNCHRONIZED:
              stack.add(st.getFirst());
              break;
            default:
              return st;
          }
        }
      }
    }

    return null;
  }

  private Set<Integer> initStatement(Statement stat) {

    HashMap<Integer, Integer> mapCount = new HashMap<>();

    List<VarExprent> condlst;

    if (stat.getExprents() == null) {

      // recurse on children statements
      List<Integer> childVars = new ArrayList<>();
      List<Exprent> currVars = new ArrayList<>();

      for (Statement st : stat.getStats()) {
        childVars.addAll(initStatement(st));

        if (st instanceof DoStatement) {
          DoStatement dost = (DoStatement)st;
          if (dost.getLooptype() != DoStatement.Type.FOR &&
            dost.getLooptype() != DoStatement.Type.FOR_EACH &&
            dost.getLooptype() != DoStatement.Type.INFINITE) {
            currVars.add(dost.getConditionExprent());
          }
        }
        else if (st instanceof CatchAllStatement) {
          CatchAllStatement fin = (CatchAllStatement)st;
          if (fin.isFinally() && fin.getMonitor() != null) {
            currVars.add(fin.getMonitor());
          }
        }
      }

      currVars.addAll(stat.getStatExprents());

      // children statements
      for (Integer index : childVars) {
        Integer count = mapCount.get(index);
        if (count == null) {
          count = 0;
        }
        mapCount.put(index, count + 1);
      }

      condlst = getAllVars(currVars);
    }
    else {
      condlst = getAllVars(stat.getExprents());
    }

    // this statement
    for (VarExprent var : condlst) {
      mapCount.put(var.getIndex(), 2);
    }


    HashSet<Integer> set = new HashSet<>(mapCount.keySet());

    // put all variables defined in this statement into the set
    for (Entry<Integer, Integer> en : mapCount.entrySet()) {
      if (en.getValue() > 1) {
        mapVarDefStatements.put(en.getKey(), stat);
      }
    }

    mapStatementVars.put(stat.id, set);

    return set;
  }

  private static List<VarExprent> getAllVars(List<Exprent> lst) {

    List<VarExprent> res = new ArrayList<>();
    List<Exprent> listTemp = new ArrayList<>();

    for (Exprent expr : lst) {
      listTemp.addAll(expr.getAllExprents(true));
      listTemp.add(expr);
    }

    for (Exprent exprent : listTemp) {
      if (exprent instanceof VarExprent) {
        res.add((VarExprent)exprent);
      }
    }

    return res;
  }

  private boolean setDefinition(Exprent expr, int index) {
    if (expr instanceof AssignmentExprent) {
      Exprent left = ((AssignmentExprent)expr).getLeft();
      if (left instanceof VarExprent) {
        VarExprent var = (VarExprent)left;
        if (var.getIndex() == index) {
          var.setDefinition(true);
          return true;
        }
      }
    }
    return false;
  }

  private void populateTypeBounds() {
    Map<VarVersionPair, VarType> mapExprentMaxTypes = varproc.getVarVersions().getTypeProcessor().getUpperBounds();
    LinkedList<Statement> stack = new LinkedList<>();
    stack.add(root);

    while (!stack.isEmpty()) {
      Statement st = stack.removeFirst();

      if (st.getExprents() != null) {
        LinkedList<Exprent> exps = new LinkedList<>();
        exps.addAll(st.getExprents());
        while (!exps.isEmpty()) {
          Exprent exp = exps.removeFirst();

          switch (exp.type) {
            case INVOCATION:
            case FIELD:
            case EXIT:
              Exprent instance = null;
              VarType newType = null;
              if (exp instanceof InvocationExprent) {
                instance = ((InvocationExprent)exp).getInstance();
                newType = new VarType(CodeType.OBJECT, 0, ((InvocationExprent)exp).getClassname());
              } else if (exp instanceof FieldExprent) {
                instance = ((FieldExprent)exp).getInstance();
                newType = new VarType(CodeType.OBJECT, 0, ((FieldExprent)exp).getClassname());
              } else if (exp instanceof ExitExprent) {
                ExitExprent exit = (ExitExprent)exp;
                if (exit.getExitType() == ExitExprent.Type.RETURN) {
                  instance = exit.getValue();
                  newType = exit.getRetType();
                }
              }

              if (newType == null || newType.typeFamily != TypeFamily.OBJECT) {
                continue;
              }
              if (newType.arrayDim == 0 && "java/lang/Object".equals(newType.value)) {
                continue; // This is dirty, but if we don't then too many things become object...
              }

              if (instance != null && instance instanceof VarExprent) {
                VarVersionPair key = ((VarExprent)instance).getVarVersionPair();
                VarType oldMax = mapExprentMaxTypes.get(key);

                if (!newType.equals(oldMax)) {
                  if (oldMax != null && oldMax.typeFamily == TypeFamily.OBJECT) {
                    // If old max is above this type in the lattice, tighten it.
                    if (oldMax.higherEqualInLatticeThan(newType)) {
                      mapExprentMaxTypes.put(key, newType);
                    }
                  } else {
                    mapExprentMaxTypes.put(key, newType);
                  }
                }
              }

              break;
            default:
              exps.addAll(exp.getAllExprents());
          }
        }
      }

      stack.addAll(st.getStats());
    }
  }

  static class VarID {
    final VarExprent var;

    VarID(VarExprent var) {
      this.var = var;
    }

    @Override
    public int hashCode() {
      return System.identityHashCode(var);
    }

    @Override
    public boolean equals(Object obj) {
      return obj instanceof VarID varID && var == varID.var;
    }
  }

  private Map<VarID, Set<VarID>> getVarExprentSources() {
    // Do an ssau analysis to find the sources of variables
    SSAUConstructorSparseEx ssau = new SSAUConstructorSparseEx();
    try {
      ssau.splitVariables(root, mt);
    } catch (NullPointerException t) {
      // Can happen when something is wrong with variables ...

      StackVarsProcessor.setVersionsToNull(root);
      return null;
    }

    Map<VarVersionPair, VarID> lookup = new HashMap<>();
    findAllVarExprents(root, lookup);

    Map<VarID, Set<VarID>> sources = new HashMap<>();
    for (VarVersionNode node : ssau.getSsuVersions().nodes) {
      VarID target = lookup.get(node.asPair());
      if (target == null) {
        continue;
      }

      Set<VarID> sourceVars = new HashSet<>();

      for (VarVersionNode predecessor : node.getPredecessors()) {
        VarID source = lookup.get(predecessor.asPair());
        if (source != null) {
          sourceVars.add(source);
        }
      }

      if (node.phantomNode != null) {
        VarID source = lookup.get(node.phantomNode.asPair());
        if (source != null) {
          sourceVars.add(source);
        }
      }

      if (!sourceVars.isEmpty()) {
        sources.put(target, sourceVars);
      }
    }

    StackVarsProcessor.setVersionsToNull(root);

    return sources;
  }

  private static void findAllVarExprents(Statement stat, Map<VarVersionPair, VarID> lookup) {
    for (Exprent exprent : stat.getVarDefinitions()) {
      if (exprent instanceof VarExprent varExprent) {
        lookup.put(new VarVersionPair(varExprent), new VarID(varExprent));
      }
    }
    List<Exprent> lst = stat.getExprents();
    if (lst != null) {
      for (Exprent exprent : lst) {
        for (Exprent exp : exprent.getAllExprents(true, true)) {
          if (exp instanceof VarExprent varExprent) {
            lookup.put(new VarVersionPair(varExprent), new VarID(varExprent));
          }
        }
      }
    }

    for (Statement subStat : stat.getStats()) {
      findAllVarExprents(subStat, lookup);
    }
  }

  private void compareVarExprentSources(
    Map<VarID, Set<VarID>> oldSources,
    Map<VarID, Set<VarID>> newSources
  ) {
    if (newSources == null) return;

    for (var oldEntry : oldSources.entrySet()) {
      Set<VarID> oldSet = oldEntry.getValue();
      Set<VarID> newSet = newSources.get(oldEntry.getKey());

      // Check if sets match
      if (!Objects.equals(oldSet, newSet)) {
        root.addComment("$VF: Variable merging failed for merge " + oldEntry.getKey().var + ". Code has semantic differences!");
      }
    }

    for (var newVar : newSources.keySet()) {
      if (!oldSources.containsKey(newVar)) {
        root.addComment("$VF: Variable merging added a var? " + newVar.var);
      }
    }
  }

  private void mergeVars(RootStatement stat) {
    Map<Integer, VarVersionPair> parent = new HashMap<>();
    Map<VarVersionPair, VarVersionPair> parentOrigins = new HashMap<>();
    MethodDescriptor md = MethodDescriptor.parseDescriptor(mt.getDescriptor());

    int index = 0;
    // this var
    if (!mt.hasModifier(CodeConstants.ACC_STATIC)) {
      VarVersionPair receiver = new VarVersionPair(index, 0);
      parent.put(index, receiver);
      putOrigin(parentOrigins, receiver);
      index++;
    }

    for (VarType var : md.params) {
      VarVersionPair parameter = new VarVersionPair(index, 0);
      parent.put(index, parameter);
      putOrigin(parentOrigins, parameter);
      index += var.stackSize;
    }

    populateTypeBounds();
    nullAssignmentDefinitions.clear();
    assignmentUseUpperBounds.clear();
    VariableOccurrences occurrences = new VariableOccurrences(stat, exprent -> {
      VarVersionPair nullDefinition = getNullAssignmentDefinition(exprent);
      if (nullDefinition != null) nullAssignmentDefinitions.add(nullDefinition);
      if (exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
        collectAssignmentUseUpperBound(assignment.getRight(), assignment.getLeft().getExprType());
      }
    });

    Map<VarID, Set<VarID>> sources = null;
    if (DecompilerContext.getOption(IFernflowerPreferences.VERIFY_PRE_POST_VARIABLE_MERGES)) {
      sources = getVarExprentSources();
    }

    Map<VarVersionPair, VarVersionPair> denylist = new HashMap<>();
    VPPEntry remap = mergeVars(stat, parent, parentOrigins, new HashMap<>(), new HashMap<>(), denylist);
    while (remap != null) {
      if (!remapVar(occurrences, remap.getKey(), remap.getValue(), remap.getMergedTypeOverride())) {
        denylist.put(remap.getKey(), remap.getValue());
      }

      remap = mergeVars(stat, parent, parentOrigins, new HashMap<>(), new HashMap<>(), denylist);
    }

    if (sources != null) {
      Map<VarID, Set<VarID>> newSources = getVarExprentSources();
      compareVarExprentSources(sources, newSources);
    }
  }

  // Match the ordering consumed by isVarReadFirst: headers, then children.
  private static List<Object> getSequentialObjects(Statement stat) {
    ArrayList<Object> lst = new ArrayList<>();
    lst.addAll(stat.getStatExprents());
    lst.addAll(stat.getStats());
    return lst;
  }


  private VPPEntry mergeVars(
    Statement stat,
    Map<Integer, VarVersionPair> parent,
    Map<VarVersionPair, VarVersionPair> parentOrigins,
    Map<Integer, VarVersionPair> leaked,
    Map<VarVersionPair, VarVersionPair> leakedOrigins,
    Map<VarVersionPair, VarVersionPair> denylist
  ) {
    Map<Integer, VarVersionPair> this_vars = new HashMap<>();
    Map<VarVersionPair, VarVersionPair> thisOrigins = new HashMap<>(parentOrigins);
    if (parent.size() > 0)
      this_vars.putAll(parent);

    if (stat.getVarDefinitions().size() > 0) {
      for (int x = 0; x < stat.getVarDefinitions().size(); x++) {
        Exprent exp = stat.getVarDefinitions().get(x);
        if (exp instanceof VarExprent) {
          VarExprent var = (VarExprent)exp;
          Integer index = varproc.getVarOriginalIndex(var.getIndex());
          if (index != null) {
            VarVersionPair current = new VarVersionPair(var);
            VarVersionPair existing = getExistingVar(this_vars, thisOrigins, index, current);

            if (existing != null && canMergeWithExistingVar(index, current, existing)) {
              VarType mergedTypeOverride = getExistingNullAssignmentMergeType(current, existing);
              if (!existing.equals(denylist.get(current)) && canMergeTypes(current, existing, mergedTypeOverride)) {
                return new VPPEntry(var, existing, mergedTypeOverride);
              }
            }

            this_vars.put(index, current);
            leaked.put(index, current);
            putOrigin(thisOrigins, current);
            putOrigin(leakedOrigins, current);
          } else {
            RootStatement root = stat.getTopParent();

            root.addComment("$VF: One or more variable merging failures!", true);
          }
        }
      }
    }

    Map<Integer, VarVersionPair> scoped = null;
    switch (stat.type) { // These are the type of statements that leak vars
      case BASIC_BLOCK:
      case GENERAL:
      case ROOT:
      case SEQUENCE:
        scoped = leaked;
    }

    if (stat.getExprents() == null) {
      List<Object> objs = getSequentialObjects(stat);
      for (int i = 0; i < objs.size(); i++) {
        Object obj = objs.get(i);
        if (obj instanceof Statement) {
          Statement st = (Statement)obj;

          Map<Integer, VarVersionPair> leaked_n = new HashMap<>();
          Map<VarVersionPair, VarVersionPair> leakedOriginsN = new HashMap<>();
          VPPEntry remap = mergeVars(st, this_vars, thisOrigins, leaked_n, leakedOriginsN, denylist);

          if (remap != null) {
            return remap;
          }

          if (!leaked_n.isEmpty() || !leakedOriginsN.isEmpty()) {
            if (stat instanceof IfStatement) {
              IfStatement ifst = (IfStatement)stat;
              if (obj == ifst.getIfstat() || obj == ifst.getElsestat()) {
                leaked_n.clear(); // Force no leaking at the end of if blocks
                leakedOriginsN.clear();
                // We may need to do this for Switches as well.. But havent run into that issue yet...
              }
              else if (obj == ifst.getFirst()) {
                leaked.putAll(leaked_n); //First is outside the scope so leak!
                leakedOrigins.putAll(leakedOriginsN);
              }
            } else if (stat instanceof SwitchStatement ||
                       stat instanceof SynchronizedStatement) {
              if (obj == stat.getFirst()) {
                leaked.putAll(leaked_n); //First is outside the scope so leak!
                leakedOrigins.putAll(leakedOriginsN);
              }
              else {
                leaked_n.clear();
                leakedOriginsN.clear();
              }
            }
            else if (stat instanceof CatchStatement || stat instanceof CatchAllStatement) {
              leaked_n.clear(); // Catches can't leak anything
              leakedOriginsN.clear();
            }
            this_vars.putAll(leaked_n);
            thisOrigins.putAll(leakedOriginsN);
          }
        }
        else if (obj instanceof Exprent) {
          VPPEntry ret = processExprent((Exprent)obj, this_vars, thisOrigins, scoped, scoped == null ? null : leakedOrigins, denylist);
          if (ret != null && isVarReadFirst(ret.getValue(), stat, i + 1) && canMergeTypes(ret.getKey(), ret.getValue(), ret.getMergedTypeOverride())) {
            return ret;
          }
        }
      }
    }
    else {
      List<Exprent> exps = stat.getExprents();
      for (int i = 0; i < exps.size(); i++) {
        Exprent exp = exps.get(i);
        VPPEntry ret = processExprent(exp, this_vars, thisOrigins, scoped, scoped == null ? null : leakedOrigins, denylist);
        if (ret != null && !isVarReadFirst(ret.getValue(), stat, i + 1)) {
          // Only merge when we can derive a valid shared type for the remap pair.
          if (canMergeTypes(ret.getKey(), ret.getValue(), ret.getMergedTypeOverride())) {
            // TODO: this only checks for totally disjoint types, there are instances where merging is incorrect with primitives

            boolean ok = true;
            if (DecompilerContext.getOption(IFernflowerPreferences.VERIFY_VARIABLE_MERGES)) {
              if (exp instanceof AssignmentExprent) {
                AssignmentExprent assign = (AssignmentExprent) exp;
                if (assign.getLeft() instanceof VarExprent) {
                  VarExprent var = (VarExprent) assign.getLeft();

                  if (var.getIndex() == ret.getKey().var) {
                    // Matched:
                    //   var<ret.key.idx> = ...

                    if (assign.getRight().containsVar(ret.getValue())) {
                      // What we're remapping to is used in the rhs!
                      // We need to iterate down the scope tree to make sure the old var isn't used anywhere else.

                      if (isVarReadRemote(identifyParent(stat), ret.getKey(), false, stat)) {
                        // The var is used elsewhere, we can't remap it
                        ok = false;
                      }
                    } else {
                      if (isVarReadRemote(identifyParent(stat), ret.getKey(), true, stat)) {
                        // The var is used elsewhere, we can't remap it
                        ok = false;
                      }
                    }
                  }
                }
              }
            }

            if (ok) {
              return ret;
            }
          }
        }
      }
    }
    return null; // We made it with no remaps!!!!!!!
  }

  private static Statement identifyParent(Statement stat) {
    Statement parent = stat.getParent();

    if (parent instanceof IfStatement || parent instanceof SwitchStatement) {
      if (parent.getBasichead() == stat) {
        return parent.getParent();
      }
    }

    // TODO: do ?

    return parent;
  }

  private static boolean isVarReadRemote(Statement stat, VarVersionPair var, boolean checkAssign, Statement... filter) {
    for (Statement st : stat.getStats()) {
      if (isVarReadRemote(st, var, checkAssign, filter)) {
        return true;
      }
    }

    if (ArrayHelper.containsByRef(filter, stat)) {
      return false;
    }

    if (stat instanceof BasicBlockStatement) {
      if (checkAssign) {
        for (Exprent ex : stat.getExprents()) {
          for (Exprent e : ex.getAllExprents(true, true)) {
            if (e instanceof AssignmentExprent) {
              AssignmentExprent assign = (AssignmentExprent)e;
              if (assign.getLeft() instanceof VarExprent) {
                VarExprent var2 = (VarExprent)assign.getLeft();
                if (var2.getIndex() == var.var) {
                  return true;
                }
              }
            }

            if (e instanceof FunctionExprent) {
              FunctionExprent func = (FunctionExprent)e;
              if (func.getFuncType().isPPMM()) {
                if (func.getLstOperands().get(0) instanceof VarExprent) {
                  VarExprent var2 = (VarExprent)func.getLstOperands().get(0);
                  if (var2.getIndex() == var.var) {
                    return true;
                  }
                }
              }
            }
          }
        }
      } else {
        for (Exprent ex : stat.getExprents()) {
          if (ex.containsVar(var)) {
            return true;
          }
        }
      }
    }


    return false;
  }

  private VPPEntry processExprent(
    Exprent exp,
    Map<Integer, VarVersionPair> thisVars,
    Map<VarVersionPair, VarVersionPair> thisOrigins,
    Map<Integer, VarVersionPair> leaked,
    Map<VarVersionPair, VarVersionPair> leakedOrigins,
    Map<VarVersionPair, VarVersionPair> denylist
  ) {
    VarExprent var = null;

    if (exp instanceof AssignmentExprent) {
      AssignmentExprent ass = (AssignmentExprent)exp;
      if (!(ass.getLeft() instanceof VarExprent)) {
        return null;
      }

      var = (VarExprent)ass.getLeft();
    }
    else if (exp instanceof VarExprent) {
      var = (VarExprent)exp;
    }

    if (var == null) {
      return null;
    }

    if (!var.isDefinition()) {
      return null;
    }

    Integer index = varproc.getVarOriginalIndex(var.getIndex());
    if (index != null) {
      VarVersionPair old = new VarVersionPair(var);
      VarVersionPair origin = varproc.getVarOriginalPair(old.var);
      VarVersionPair exactOriginVar = origin == null ? null : thisOrigins.get(origin);
      VarVersionPair new_ = exactOriginVar != null ? exactOriginVar : thisVars.get(index);
      if (new_ != null && canMergeWithExistingVar(index, old, new_)) {
        VarVersionPair deny = denylist.get(old);
        if (deny == null || !deny.equals(new_)) {
          if (exactOriginVar == null && origin != null) {
            // Repeated SSA passes can split one earlier variable into multiple Java
            // indices. Preserve that exact origin even if the raw-slot merge below
            // is rejected, so later fragments do not see an unrelated slot lifetime.
            thisOrigins.put(origin, old);
            if (leakedOrigins != null) {
              leakedOrigins.put(origin, old);
            }
          }
          return new VPPEntry(var, new_, getNullAssignmentMergeType(exp, old, new_));
        }
      }

      thisVars.put(index, old);
      if (leaked != null) {
        leaked.put(index, old);
      }
      putOrigin(thisOrigins, old);
      if (leakedOrigins != null) {
        putOrigin(leakedOrigins, old);
      }
    }

    return null;
  }

  private VarVersionPair getExistingVar(
    Map<Integer, VarVersionPair> varsBySlot,
    Map<VarVersionPair, VarVersionPair> varsByOrigin,
    int originalIndex,
    VarVersionPair current
  ) {
    VarVersionPair origin = varproc.getVarOriginalPair(current.var);
    VarVersionPair exact = origin == null ? null : varsByOrigin.get(origin);
    return exact != null ? exact : varsBySlot.get(originalIndex);
  }

  private void putOrigin(Map<VarVersionPair, VarVersionPair> varsByOrigin, VarVersionPair variable) {
    VarVersionPair origin = varproc.getVarOriginalPair(variable.var);
    if (origin != null) {
      varsByOrigin.put(origin, variable);
    }
  }

  private VarType getNullAssignmentMergeType(Exprent exp, VarVersionPair source, VarVersionPair target) {
    if (isNullAssignmentDefinition(exp)) {
      // A null-only local is assignable to the existing reference type. Do not let
      // its normalized Object type widen a concrete array/object slot merge.
      VarType targetType = varproc.getVarType(target);
      return targetType != null && targetType.typeFamily == TypeFamily.OBJECT ? targetType : null;
    }

    return getExistingNullAssignmentMergeType(source, target);
  }

  private VarType getExistingNullAssignmentMergeType(VarVersionPair source, VarVersionPair target) {
    if (!nullAssignmentDefinitions.contains(target)) {
      return null;
    }

    VarType sourceType = varproc.getVarType(source);
    return isSpecificReferenceType(sourceType) && satisfiesAssignmentUseUpperBounds(sourceType, target) ? sourceType : null;
  }

  private void collectAssignmentUseUpperBound(Exprent exprent, VarType upperBound) {
    if (upperBound == null || upperBound.type == CodeType.UNKNOWN) {
      return;
    }

    if (exprent instanceof VarExprent var) {
      assignmentUseUpperBounds.computeIfAbsent(new VarVersionPair(var), key -> new HashSet<>()).add(upperBound);
    }
    else if (exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
      collectAssignmentUseUpperBound(assignment.getRight(), upperBound);
    }
  }

  private static boolean isNullAssignmentDefinition(Exprent exprent) {
    return getNullAssignmentDefinition(exprent) != null;
  }

  private static VarVersionPair getNullAssignmentDefinition(Exprent exprent) {
    return exprent instanceof AssignmentExprent assignment &&
      assignment.getLeft() instanceof VarExprent var &&
      var.isDefinition() &&
      assignment.getRight() instanceof ConstExprent constExpr &&
      constExpr.isNull()
      ? new VarVersionPair(var)
      : null;
  }

  private void mergeNullAssignmentDefinitions(VarVersionPair from, VarVersionPair to, VarType merged) {
    boolean mergedIsStillOnlyNull = VarType.VARTYPE_OBJECT.equals(merged) &&
      nullAssignmentDefinitions.contains(from) &&
      nullAssignmentDefinitions.contains(to);
    nullAssignmentDefinitions.remove(from);
    nullAssignmentDefinitions.remove(to);
    if (mergedIsStillOnlyNull) {
      nullAssignmentDefinitions.add(to);
    }
  }

  private void mergeAssignmentUseUpperBounds(VarVersionPair from, VarVersionPair to) {
    Set<VarType> fromTypes = assignmentUseUpperBounds.remove(from);
    if (fromTypes == null || fromTypes.isEmpty()) {
      return;
    }

    assignmentUseUpperBounds.computeIfAbsent(to, key -> new HashSet<>()).addAll(fromTypes);
  }

  private boolean satisfiesAssignmentUseUpperBounds(VarType mergedType, VarVersionPair pair) {
    Set<VarType> upperBounds = assignmentUseUpperBounds.get(pair);
    if (upperBounds == null || upperBounds.isEmpty()) {
      return true;
    }

    for (VarType upperBound : upperBounds) {
      if (!upperBound.higherEqualInLatticeThan(mergedType)) {
        return false;
      }
    }
    return true;
  }

  private boolean canMergeWithExistingVar(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    // A copy of the current receiver is a distinct source value even when its
    // bytecode slot previously held an assignable parameter or local. Reusing
    // that source variable would retain the old declared type while changing
    // the value's member-resolution semantics.
    if (varproc.isReceiverEquivalent(current) != varproc.isReceiverEquivalent(existing)) {
      return false;
    }

    // A parameter binding describes the value supplied by the caller, not every
    // unrelated value an obfuscated method later stores in the same JVM slot.
    if (isOverwrittenSemanticParameter(originalIndex, current, existing)) {
      return false;
    }

    if (!isOverwrittenReceiverSlot(originalIndex, current, existing)) {
      return !isIncompatibleOverwrittenParameterSlot(originalIndex, current, existing);
    }

    // Slot 0 can be reassigned in obfuscated bytecode. Keep those locals distinct from the Java receiver.
    return current.equals(existing);
  }

  private boolean isOverwrittenSemanticParameter(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    return !current.equals(existing)
      && varproc.getParams().contains(existing)
      && semanticParameterSlots.contains(originalIndex);
  }

  private Set<Integer> findSemanticParameterSlots() {
    SemanticMappings mappings = DecompilerContext.getContextProperty(DecompilerContext.SEMANTIC_MAPPINGS);
    if (mappings == null) {
      return Set.of();
    }

    Set<Integer> slots = new HashSet<>();
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(mt.getDescriptor());
    MemberKey method = new MemberKey(mt.getClassQualifiedName(), mt.getName(), mt.getDescriptor());
    int slot = mt.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      if (mappings.hasParameterSemantics(method, parameter)) {
        slots.add(slot);
      }
      slot += descriptor.params[parameter].stackSize;
    }
    return Set.copyOf(slots);
  }

  private boolean isIncompatibleOverwrittenParameterSlot(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    if (current.equals(existing) || !varproc.getParams().contains(existing)) {
      return false;
    }

    VarType parameterType = varproc.getParameterTypeByOriginalIndex(originalIndex);
    VarType currentType = varproc.getVarType(current);
    return parameterType != null &&
      currentType != null &&
      currentType.type != CodeType.UNKNOWN &&
      !isAssignableToDeclaredParameter(parameterType, currentType);
  }

  private static boolean isAssignableToDeclaredParameter(VarType parameterType, VarType currentType) {
    if (parameterType.type == CodeType.BOOLEAN || currentType.type == CodeType.BOOLEAN) {
      return parameterType.type == currentType.type;
    }

    return parameterType.higherCrossFamilyThan(currentType, true);
  }

  private Map<VarVersionPair, Set<VarType>> collectLegacySlotTypeEvidence() {
    Map<VarVersionPair, Set<VarType>> observedTypes = new HashMap<>();
    StatementIterator.iterate(root, exprent -> {
      if (exprent instanceof VarExprent var && var.getVersion() >= 0 && !var.getStackMapTypes().isEmpty()) {
        observedTypes.computeIfAbsent(new VarVersionPair(var), k -> new HashSet<>()).addAll(var.getStackMapTypes());
      }
      return 0;
    });
    return observedTypes;
  }

  private void mergeLegacySlotTypeEvidence(VarVersionPair from, VarVersionPair to) {
    if (!j2meStrictSlotMerge || legacySlotTypeEvidence.isEmpty()) {
      return;
    }

    Set<VarType> fromTypes = legacySlotTypeEvidence.remove(from);
    if (fromTypes == null || fromTypes.isEmpty()) {
      return;
    }

    legacySlotTypeEvidence.computeIfAbsent(to, key -> new HashSet<>()).addAll(fromTypes);
  }

  private boolean hasIncompatibleLegacySlotTypes(VarVersionPair from, VarVersionPair to) {
    if (legacySlotTypeEvidence.isEmpty()) {
      return false;
    }

    Set<VarType> fromTypes = legacySlotTypeEvidence.get(from);
    Set<VarType> toTypes = legacySlotTypeEvidence.get(to);
    if (fromTypes == null || fromTypes.isEmpty() || toTypes == null || toTypes.isEmpty()) {
      return false;
    }

    for (VarType fromType : fromTypes) {
      for (VarType toType : toTypes) {
        if (!areSlotTypesCompatible(fromType, toType)) {
          return true;
        }
      }
    }

    return false;
  }

  private boolean areSlotTypesCompatible(VarType first, VarType second) {
    if (first == null || second == null) {
      return true;
    }

    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) {
      return true;
    }

    boolean firstReference = first.typeFamily == TypeFamily.OBJECT;
    boolean secondReference = second.typeFamily == TypeFamily.OBJECT;

    if (first.type == CodeType.NULL || second.type == CodeType.NULL) {
      return firstReference && secondReference;
    }

    if (firstReference != secondReference) {
      return false;
    }

    if (!firstReference) {
      return first.type == second.type;
    }

    if (first.equals(VarType.VARTYPE_OBJECT) || second.equals(VarType.VARTYPE_OBJECT)) {
      return true;
    }

    if (first.arrayDim != second.arrayDim) {
      return false;
    }

    if (first.arrayDim > 0) {
      return first.equals(second);
    }

    if (first.equals(second)) {
      return true;
    }

    return DecompilerContext.getStructContext().instanceOf(first.value, second.value)
      || DecompilerContext.getStructContext().instanceOf(second.value, first.value);
  }

  private boolean canMergeTypes(VarVersionPair from, VarVersionPair to) {
    return canMergeTypes(from, to, null);
  }

  private boolean canMergeTypes(VarVersionPair from, VarVersionPair to, VarType mergedTypeOverride) {
    if (j2meStrictSlotMerge && hasIncompatibleLegacySlotTypes(from, to)) {
      return false;
    }

    VarType mergedType = mergedTypeOverride != null ? mergedTypeOverride : getMergedType(from, to);
    if (mergedType == null) {
      return false;
    }

    VarType fromType = varproc.getVarType(from);
    VarType toType = varproc.getVarType(to);

    // A missing/broad frame must not erase a boundary already established by
    // inference. Merging unrelated reference lifetimes into Object introduces casts
    // and makes slot reuse harder to reconstruct without providing a Java-level alias.
    if (j2meStrictSlotMerge && !areSlotTypesCompatible(fromType, toType)) {
      return false;
    }

    if (!sameOrUnknownTypeFamily(fromType, toType)) {
      return false;
    }

    if (hasConflictingConcretePrimitiveTypes(fromType, toType)) {
      return false;
    }

    return isLatticeCompatible(mergedType, fromType)
      && isLatticeCompatible(mergedType, toType)
      && (mergedTypeOverride == null || satisfiesUpperBounds(mergedType, from, to));
  }

  private boolean satisfiesUpperBounds(VarType mergedType, VarVersionPair from, VarVersionPair to) {
    Map<VarVersionPair, VarType> upperBounds = varproc.getVarVersions().getTypeProcessor().getUpperBounds();
    VarType fromMax = upperBounds.get(from);
    VarType toMax = upperBounds.get(to);

    return (fromMax == null || fromMax.higherEqualInLatticeThan(mergedType)) &&
           (toMax == null || toMax.higherEqualInLatticeThan(mergedType));
  }

  private static boolean hasConflictingConcretePrimitiveTypes(VarType first, VarType second) {
    if (first == null || second == null) {
      return false;
    }

    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) {
      return false;
    }

    if (first.typeFamily == TypeFamily.OBJECT || second.typeFamily == TypeFamily.OBJECT) {
      return false;
    }

    return first.type != second.type;
  }

  private static boolean sameOrUnknownTypeFamily(VarType first, VarType second) {
    if (first == null || second == null) {
      return true;
    }

    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) {
      return true;
    }

    return first.typeFamily == second.typeFamily;
  }

  private static boolean isLatticeCompatible(VarType mergedType, VarType varType) {
    if (varType == null || varType.type == CodeType.UNKNOWN) {
      return true;
    }

    return mergedType.higherEqualInLatticeThan(varType) || varType.higherEqualInLatticeThan(mergedType);
  }

  private boolean isOverwrittenReceiverSlot(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    return originalIndex == 0
      && !mt.hasModifier(CodeConstants.ACC_STATIC)
      && existing.var == 0
      && existing.version == 0
      && (current.var != 0 || current.version != 0);
  }

  private boolean remapVar(VariableOccurrences occurrences, VarVersionPair from, VarVersionPair to, VarType mergedTypeOverride) {
    if (from.equals(to)) throw new IllegalStateException("Trying to merge a variable with itself: " + from);
    VarType merged = mergedTypeOverride != null ? mergedTypeOverride : getMergedType(from, to);
    if (merged == null || !occurrences.merge(from, to, merged)) return false;

    varproc.setVarType(to, merged);
    mergeNullAssignmentDefinitions(from, to, merged);
    mergeLegacySlotTypeEvidence(from, to);
    mergeAssignmentUseUpperBounds(from, to);
    return true;
  }

  private VarType getMergedType(VarVersionPair from, VarVersionPair to) {
    Map<VarVersionPair, VarType> minTypes = varproc.getVarVersions().getTypeProcessor().getLowerBounds();
    Map<VarVersionPair, VarType> maxTypes = varproc.getVarVersions().getTypeProcessor().getUpperBounds();

    return getMergedType(minTypes.get(from), minTypes.get(to), maxTypes.get(from), maxTypes.get(to));
  }

  private static boolean isSpecificReferenceType(VarType type) {
    return type != null &&
      type.typeFamily == TypeFamily.OBJECT &&
      type.type != CodeType.NULL &&
      type.type != CodeType.UNKNOWN &&
      !type.equals(VarType.VARTYPE_OBJECT);
  }

  private VarType getMergedType(VarType fromMin, VarType toMin, VarType fromMax, VarType toMax) {
    if (!j2meStrictSlotMerge && fromMin != null && fromMin.equals(toMin)) {
      return fromMin; // Short circuit this for simplicities sake
    }

    VarType type = fromMin == null ? toMin : (toMin == null ? fromMin : VarType.join(fromMin, toMin));
    if (type == null || fromMin == null || toMin == null) {
      return null; // no common supertype, skip the remapping
    }

    if (type.type == CodeType.OBJECT) {
      VarType merged = null;
      if (toMax != null) { // The target var is used in direct invocations
        if (fromMax != null) {
          // Max types are the highest class that this variable is used as a direct instance of without any casts.
          // This will pull up the to var type if the from requires a higher class type.
          // EXA: Collection -> List
          if (DecompilerContext.getStructContext().instanceOf(fromMax.value, toMax.value)) {
            merged = fromMax;
          }
        } else {
          // Pull to up to from: List -> ArrayList
          if (DecompilerContext.getStructContext().instanceOf(fromMin.value, toMax.value)) {
            merged = fromMin;
          }
        }
      } else {
        if (fromMax != null) {
          if (DecompilerContext.getStructContext().instanceOf(fromMax.value, toMin.value)) {
            merged = fromMax;
          }
        } else {
          if (DecompilerContext.getStructContext().instanceOf(toMin.value, fromMin.value)) {
            merged = toMin;
          }

          if (merged == null && DecompilerContext.getStructContext().instanceOf(fromMin.value, toMin.value)) {
            merged = toMin;
          }
        }
      }

      if (merged == null) {
        return null;
      }

      if (j2meStrictSlotMerge && !merged.higherEqualInLatticeThan(type)) {
        merged = type;
      }

      if (fromMax != null && !fromMax.higherEqualInLatticeThan(merged)) {
        return null;
      }

      if (toMax != null && !toMax.higherEqualInLatticeThan(merged)) {
        return null;
      }

      return merged;
    } else {
      // Both nonnull at this point
      if (!fromMin.equals(toMin) && !fromMin.higherInLatticeThan(toMin)) {
        // If type we're merging into the old type isn't a strict superset of the old type, we cannot merge
        return null;
      }

      // Keep primitive merges within all known upper-bound constraints.
      // This prevents cross-family remaps like Object-slot -> int-slot reuse from collapsing into one local.
      if (fromMax != null && !fromMax.higherEqualInLatticeThan(type)) {
        return null;
      }

      if (toMax != null && !toMax.higherEqualInLatticeThan(type)) {
        return null;
      }

      return type;
    }
  }

  private void propagateLVTs(Statement stat) {
    MethodDescriptor md = MethodDescriptor.parseDescriptor(mt.getDescriptor());
    Map<VarVersionPair, VarInfo> types = new LinkedHashMap<>();

    if (varproc.hasLVT()) {
      int index = 0;
      if (!mt.hasModifier(CodeConstants.ACC_STATIC)) {
        List<LocalVariable> lvt = varproc.getCandidates(index); // Some enums give incomplete lvts?
        if (lvt != null && lvt.size() > 0) {
          types.put(new VarVersionPair(index, 0), new VarInfo(lvt.get(0), null));
        }
        index++;
      }

      for (VarType var : md.params) {
        List<LocalVariable> lvt = varproc.getCandidates(index); // Some enums give incomplete lvts?
        if (lvt != null && lvt.size() > 0) {
          types.put(new VarVersionPair(index, 0), new VarInfo(lvt.get(0), null));
        }
        index += var.stackSize;
      }
    }

    findTypes(stat, types);

    Map<VarVersionPair, Pair<VarType, String>> typeNames = new LinkedHashMap<>();
    for (Entry<VarVersionPair, VarInfo> e : types.entrySet()) {
      typeNames.put(e.getKey(), Pair.of(e.getValue().getType(), e.getValue().getCast()));
    }

    Map<VarVersionPair, String> renames = this.mt.getVariableNamer().rename(typeNames);

    Set<StructMethod> methods = new HashSet<>();

    // Stuff the parent context into enclosed child methods
    StatementIterator.iterate(root, (exprent) -> {
      if (exprent instanceof NewExprent) {
        NewExprent _new = (NewExprent)exprent;
        if (_new.isAnonymous()) { //TODO: Check for Lambda here?
          ClassNode child = DecompilerContext.getClassProcessor().getMapRootClasses().get(_new.getNewType().value);
          if (child != null) {
            if (_new.isLambda()) {
              if (child.lambdaInformation.is_method_reference) {
                //methods.add(child.getWrapper().getClassStruct().getMethod(child.lambdaInformation.content_method_key));
              } else {
                methods.add(child.classStruct.getMethod(child.lambdaInformation.content_method_name, child.lambdaInformation.content_method_descriptor));
              }
            } else {
              methods.addAll(child.classStruct.getMethods());
            }
          }
        }
      }
      return 0;
    });

    // Local classes aren't added into the method body yet
    String thisKey = InterpreterUtil.makeUniqueKey(mt.getName(), mt.getDescriptor());
    for (ClassNode nested : DecompilerContext.getClassProcessor().getMapRootClasses().get(mt.getClassQualifiedName()).nested) {
      if (nested.type == ClassNode.Type.LOCAL && thisKey.equals(nested.enclosingMethod)) {
        methods.addAll(nested.classStruct.getMethods());
      }
    }

    for (StructMethod meth : methods) {
      meth.getVariableNamer().addParentContext(VarDefinitionHelper.this.mt.getVariableNamer());
    }

    Map<VarVersionPair, LocalVariable> lvts = new HashMap<>();

    for (Entry<VarVersionPair, VarInfo> e : types.entrySet()) {
      VarVersionPair idx = e.getKey();
      // skip this. we can't rename it
      if (idx.var == 0 && !mt.hasModifier(CodeConstants.ACC_STATIC)) {
        continue;
      }
      LocalVariable lvt = e.getValue().getLVT();
      String rename = renames == null ? null : renames.get(idx);

      if (rename != null) {
        varproc.setVarName(idx, rename);
      }

      if (lvt != null) {
        if (rename != null) {
          lvt = lvt.rename(rename);
        }
        varproc.setVarLVT(idx, lvt);
        lvts.put(idx, lvt);
      }
    }


    applyTypes(stat, lvts);
  }

  private void findTypes(Statement stat, Map<VarVersionPair, VarInfo> types) {
    if (stat == null) {
      return;
    }

    for (Exprent exp : stat.getVarDefinitions()) {
      findTypes(exp, types);
    }

    if (stat.getExprents() == null) {
      for (Statement st : stat.getStats()) {
        findTypes(st, types);
      }

      for (Exprent exp : stat.getStatExprents()) {
        findTypes(exp, types);
      }
    }
    else {
      for (Exprent exp : stat.getExprents()) {
        findTypes(exp, types);
      }
    }
  }

  private void findTypes(Exprent exp, Map<VarVersionPair, VarInfo> types) {
    List<Exprent> lst = exp.getAllExprents(true);
    lst.add(exp);

    for (Exprent exprent : lst) {
      if (exprent instanceof VarExprent) {
        VarExprent var = (VarExprent)exprent;
        VarVersionPair ver = new VarVersionPair(var);
        if (var.isDefinition()) {
          types.put(ver, new VarInfo(var.getLVT(), var.getVarType()));
        } else {
          VarInfo existing = types.get(ver);
          if (existing == null) {
            existing = new VarInfo(var.getLVT(), var.getVarType());
          } else if (existing.getLVT() == null && var.getLVT() != null) {
            existing = new VarInfo(var.getLVT(), existing.getType());
          }

          types.put(ver, existing);
        }
      }
    }
  }

  private void applyTypes(Statement stat, Map<VarVersionPair, LocalVariable> types) {
    if (stat == null || types.size() == 0) {
      return;
    }

    for (Exprent exp : stat.getVarDefinitions()) {
      applyTypes(exp, types);
    }

    if (stat.getExprents() == null) {
      for (Statement st : stat.getStats()) {
        applyTypes(st, types);
      }

      for (Exprent exp : stat.getStatExprents()) {
        applyTypes(exp, types);
      }
    }
    else {
      for (Exprent exp : stat.getExprents()) {
        applyTypes(exp, types);
      }
    }
  }

  private void applyTypes(Exprent exprent, Map<VarVersionPair, LocalVariable> types) {
    if (exprent == null) {
      return;
    }
    List<Exprent> lst = exprent.getAllExprents(true);
    lst.add(exprent);

    for (Exprent expr : lst) {
      if (expr instanceof VarExprent) {
        VarExprent var = (VarExprent)expr;
        LocalVariable lvt = types.get(new VarVersionPair(var));
        if (lvt != null) {
          var.setLVT(lvt);
        } else {
          System.currentTimeMillis();
        }
      }
    }
  }

  //Helper classes because Java is dumb and doesn't have a Pair<K,V> class
  private static class SimpleEntry<K, V> implements Entry<K, V> {
    private K key;
    private V value;
    public SimpleEntry(K key, V value) {
      this.key = key;
      this.value = value;
    }
    @Override public K getKey() { return key; }
    @Override public V getValue() { return value; }
    @Override
    public V setValue(V value) {
      V tmp = this.value;
      this.value = value;
      return tmp;
    }
  }
  private static class VPPEntry extends SimpleEntry<VarVersionPair, VarVersionPair> {
    private final VarType mergedTypeOverride;

    private VPPEntry(VarExprent key, VarVersionPair value) {
      this(key, value, null);
    }

    private VPPEntry(VarExprent key, VarVersionPair value, VarType mergedTypeOverride) {
      super(new VarVersionPair(key), value);
      this.mergedTypeOverride = mergedTypeOverride;
    }

    private VarType getMergedTypeOverride() {
      return mergedTypeOverride;
    }
  }

  private static class VarInfo {
    private LocalVariable lvt;
    private String cast;
    private VarType type;

    private VarInfo(LocalVariable lvt, VarType type) {
      if (lvt != null && lvt.getSignature() != null)
        this.cast = ExprProcessor.getCastTypeName(GenericType.parse(lvt.getSignature()), false);
      else if (lvt != null)
        this.cast = ExprProcessor.getCastTypeName(lvt.getVarType(), false);
      else if (type != null)
        this.cast = ExprProcessor.getCastTypeName(type, false);
      else
        this.cast = "this";
      this.lvt = lvt;
      this.type = type;
    }

    public LocalVariable getLVT() {
      return this.lvt;
    }

    public String getCast() {
      return this.cast;
    }

    public VarType getType() {
      return this.type;
    }
  }

  private static boolean isVarReadFirst(VarVersionPair var, Statement stat, int index, VarExprent... allowlist) {
    if (stat.getExprents() == null) {
      List<Object> objs = getSequentialObjects(stat);
      for (int x = index; x < objs.size(); x++) {
        Object obj = objs.get(x);
        if (obj instanceof Statement) {
          if (isVarReadFirst(var, (Statement)obj, 0, allowlist)) {
            return true;
          }
        } else if (obj instanceof Exprent) {
          if (isVarReadFirst(var, (Exprent)obj, allowlist)) {
            return true;
          }
        }
      }
    } else {
      for (int x = index; x < stat.getExprents().size(); x++) {
        if (isVarReadFirst(var, stat.getExprents().get(x), allowlist)) {
          return true;
        }
      }
    }
    return false;
  }

  private static boolean isVarReadFirst(VarVersionPair target, Exprent exp, VarExprent... allowlist) {
    AssignmentExprent assign = exp instanceof AssignmentExprent ? (AssignmentExprent)exp : null;
    FunctionExprent func = exp instanceof FunctionExprent ? (FunctionExprent)exp : null;

    if (func != null && !func.getFuncType().isPPMM()) {
      func = null;
    }

    List<Exprent> lst = exp.getAllExprents(true, true);

    for (Exprent ex : lst) {
      if (ex instanceof VarExprent) {
        VarExprent var = (VarExprent)ex;
        if (var.getIndex() == target.var && var.getVersion() == target.version) {
          boolean allowed = false;

          if (assign != null) {
            if (var == assign.getLeft()) {
              allowed = true;
            }
          }

          if (func != null) {
            if (var == func.getLstOperands().get(0)) {
              allowed = true;
            }
          }

          for (VarExprent allow : allowlist) {
            if (var == allow) {
              allowed = true;
            }
          }

          if (!allowed) {
            return true;
          }
        }
      }
    }

    return false;
  }

  private void setNonFinal(Statement stat, Set<VarVersionPair> unInitialized) {
    if (stat.getExprents() != null && !stat.getExprents().isEmpty()) {
      for (Exprent exp : stat.getExprents()) {
        if (exp instanceof VarExprent) {
          unInitialized.add(new VarVersionPair((VarExprent)exp));
        }
        else {
          setNonFinal(exp, unInitialized);
        }
      }
    }

    if (!stat.getVarDefinitions().isEmpty()) {
      if (stat instanceof DoStatement) {
        for (Exprent var : stat.getVarDefinitions()) {
          unInitialized.add(new VarVersionPair((VarExprent)var));
        }
      }
    }

    if (stat instanceof DoStatement) {
      DoStatement dostat = (DoStatement)stat;
      if (dostat.getInitExprentList() != null) {
        setNonFinal(dostat.getInitExprent(), unInitialized);
      }
      if (dostat.getIncExprentList() != null) {
        setNonFinal(dostat.getIncExprent(), unInitialized);
      }
    }
    else if (stat instanceof IfStatement) {
      IfStatement ifstat = (IfStatement)stat;
      if (ifstat.getIfstat() != null && ifstat.getElsestat() != null) {
        setNonFinal(ifstat.getFirst(), unInitialized);
        setNonFinal(ifstat.getIfstat(), new HashSet<>(unInitialized));
        setNonFinal(ifstat.getElsestat(), unInitialized);
        return;
      }
    }

    for (Statement st : stat.getStats()) {
      setNonFinal(st, unInitialized);
    }
  }

  private void setNonFinal(Exprent exp, Set<VarVersionPair> unInitialized) {
    VarExprent var = null;

    if (exp == null) {
      return;
    }

    if (exp instanceof AssignmentExprent) {
      AssignmentExprent assign = (AssignmentExprent)exp;
      if (assign.getLeft() instanceof VarExprent) {
        var = (VarExprent)assign.getLeft();
      }
    }
    else if (exp instanceof FunctionExprent) {
      FunctionExprent func = (FunctionExprent)exp;
      if (func.getFuncType().isPPMM()) {
        if (func.getLstOperands().get(0) instanceof VarExprent) {
          var = (VarExprent)func.getLstOperands().get(0);
        }
      }
    }

    if (var != null && !var.isDefinition() && !unInitialized.remove(var.getVarVersionPair())) {
      var.getProcessor().setVarFinal(var.getVarVersionPair(), FinalType.NON_FINAL);
    }

    for (Exprent ex : exp.getAllExprents()) {
      setNonFinal(ex, unInitialized);
    }
  }

  public Map<VarVersionPair, String> getClashingNames() {
    return clashingNames;
  }
}
