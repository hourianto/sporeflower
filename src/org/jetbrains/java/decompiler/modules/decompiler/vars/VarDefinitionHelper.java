// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.VarNamesCollector;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.ExprProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarTypeProcessor.FinalType;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructLocalVariableTableAttribute.LocalVariable;
import org.jetbrains.java.decompiler.struct.attr.StructMethodParametersAttribute;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.struct.gen.generics.GenericType;
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
  private final Map<VarVersionPair, String> clashingNames = new HashMap<>();
  private final VariableCoalescer coalescer;

  public VarDefinitionHelper(RootStatement root, StructMethod mt, VarProcessor varproc) {
    mapVarDefStatements = new HashMap<>();
    mapStatementVars = new HashMap<>();
    implDefVars = new HashSet<>();

    this.varproc = varproc;
    this.root = root;
    this.mt = mt;
    this.coalescer = new VariableCoalescer(root, mt, varproc);

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

    coalescer.coalesce();

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

      VarVersionPair pair = new VarVersionPair(index, 0);
      VarProcessor.SemanticName semanticName = varproc.getSemanticName(pair);
      varproc.setVarName(pair, semanticName == null ? vc.getFreeName(index) : vc.getFreeName(semanticName.name()));

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

    coalescer.coalesce();
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
      if (lvt == null && varproc.getSemanticName(idx) != null) rename = null;

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
