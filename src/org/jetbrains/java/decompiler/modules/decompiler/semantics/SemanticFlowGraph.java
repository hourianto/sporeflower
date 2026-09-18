package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ContainerSemantics;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticFacts.*;

/**
 * A method's value graph, independent of printable local names. Producer alternatives
 * are solved first. Consumer requirements then travel only through value-preserving
 * edges; an opaque reaching definition cannot acquire a meaning from a later use.
 * Rendering observes the reconciled result after both analyses have converged.
 */
final class SemanticFlowGraph implements SemanticUses.Sink {
  private static final class Node {
    final Exprent expression;
    final Set<SemanticLocalFlow.Definition> sources;
    final Set<Node> readers = new LinkedHashSet<>();
    SemanticFacts facts = BOTTOM;
    SemanticFacts produced = BOTTOM;
    SemanticFacts required = BOTTOM;
    boolean loop;
    boolean queued;
    Node(Exprent expression) {
      this.expression = expression;
      this.sources = new LinkedHashSet<>();
    }
  }
  private final SemanticAnalysis analysis;
  private final Map<Exprent, Node> expressions = new IdentityHashMap<>();
  private final Map<VarVersionPair, SemanticFacts> parameters = new LinkedHashMap<>();
  private final Map<SemanticLoopIndex, Node> loops = new IdentityHashMap<>();
  private final List<Node> nodes = new ArrayList<>();
  private final Deque<Node> pending = new ArrayDeque<>();
  private final SemanticHeap heap;
  private Node reading;
  private boolean contextual;

  SemanticFlowGraph(SemanticAnalysis analysis) {
    this.analysis = analysis;
    for (Exprent expression : analysis.expressions) register(expression);
    heap = new SemanticHeap(analysis);
  }

  private void register(Exprent expression) {
    Node node;
    SemanticLoopIndex loop = analysis.context.boundedLoop(expression);
    if (loop != null) {
      node = loops.get(loop);
      if (node == null) {
        node = new Node(null);
        node.loop = true;
        loops.put(loop, node);
        nodes.add(node);
      }
    } else {
      node = new Node(expression);
      if (expression instanceof VarExprent)
        node.sources.addAll(analysis.locals.sources(expression));
      nodes.add(node);
    }
    expressions.put(expression, node);
  }

  void parameter(VarVersionPair variable, SemanticFacts facts) {
    parameters.put(variable, facts);
  }

  SemanticFacts facts(Exprent expression) {
    Node node = expressions.get(expression);
    if (node == null)
      return UNKNOWN;
    if (reading != null)
      node.readers.add(reading);
    return node.facts;
  }

  SemanticFacts requirements(Exprent expression) {
    Node node = expressions.get(expression);
    return node == null ? BOTTOM : node.required;
  }

  void solveProducers() {
    for (Node node : nodes) enqueue(node);
    solveForward();
  }

  private boolean solveForward() {
    boolean changed = false;
    while (!pending.isEmpty()) {
      Node node = pending.removeFirst();
      node.queued = false;
      reading = node;
      SemanticFacts facts;
      if (node.loop) {
        // The body reads a proven induction value. Its initialization and update
        // are arithmetic, not semantic definitions to rename retroactively.
        facts = BOTTOM;
      } else if (node.expression instanceof VarExprent) {
        facts = node.sources.isEmpty() ? UNKNOWN : BOTTOM;
        for (SemanticLocalFlow.Definition source : node.sources) {
          facts = facts.merge(source.value == null ? parameters.getOrDefault(source.incoming, UNKNOWN) : facts(source.value));
        }
      } else if (node.expression instanceof NewExprent creation
        && (creation.getNewType().arrayDim > 0 || SemanticHeap.isContainer(creation))) {
        facts = heap.facts(creation);
      } else {
        facts = analysis.computeFacts(node.expression);
      }
      reading = null;
      node.produced = facts;
      if (contextual)
        facts = reconcile(node, facts);
      if (!facts.equals(node.facts)) {
        changed = true;
        node.facts = facts;
        for (Node reader : node.readers) enqueue(reader);
      }
    }
    return changed;
  }

  private SemanticFacts reconcile(Node node, SemanticFacts produced) {
    // A return/parameter shape can describe a fresh locally assembled array even
    // when its elements came from unannotated computations. This is a boundary
    // contract, not an inference that all arrays with similar values share it.
    if (produced.unknown() && !node.required.arrays().isEmpty() && localArray(node)) {
      produced = new SemanticFacts(produced.domains(), produced.arrays(), produced.containers(), produced.dependent(), false);
    }
    return produced.require(node.required);
  }

  private boolean localArray(Node node) {
    return node.expression != null && heap.isLocal(node.expression);
  }

  public void domain(Exprent expression, String domain, VarType type) {
    if (domain != null)
      require(expression, SemanticFacts.of(domain, null));
  }

  public void array(Exprent expression, ArraySemantics shape) {
    if (shape != null)
      require(expression, SemanticFacts.of(null, shape));
  }

  public void container(Exprent expression, ContainerSemantics shape) {
    if (shape != null)
      require(expression, SemanticFacts.declaration(null, null, shape));
  }

  private void require(Exprent expression, SemanticFacts requirement) {
    Node node = expressions.get(expression);
    if (node == null
      || requirement.domains().isEmpty() && requirement.arrays().isEmpty() && requirement.containers().isEmpty()
        && requirement.dependent().isEmpty())
      return;
    SemanticFacts merged = node.required.require(requirement);
    if (!merged.equals(node.required)) {
      node.required = merged;
      enqueue(node);
    }
  }

  void solveRequirements(SemanticUses uses) {
    // A dependent consumer can become meaningful only after another boundary
    // supplies a table/container shape. Revisit uses and backward edges until
    // both evidence and requirements stabilize, rather than stopping after one
    // forward/backward round. Requirements only accumulate finite contracts.
    boolean changed;
    do {
      uses.visit();
      // A join with one established meaning and neutral literal alternatives
      // supplies that meaning even without an explicit downstream contract.
      // Keep this at the read: unrelated overwrites do not share its context.
      for (Node node : nodes) {
        if (node.expression instanceof VarExprent && !node.produced.unknown())
          require(node.expression, node.produced);
      }
      for (Node node : nodes)
        if (!node.required.isEmpty())
          enqueue(node);
      while (!pending.isEmpty()) {
        Node node = pending.removeFirst();
        node.queued = false;
        SemanticFacts required = reconcile(node, node.produced);
        if (node.loop)
          continue;
        if (node.expression instanceof VarExprent) {
          if (node.produced.unknown() && !localArray(node))
            continue;
          for (SemanticLocalFlow.Definition source : node.sources) {
            if (source.value != null)
              require(source.value, required);
          }
        } else {
          transferRequirement(node.expression, required);
        }
      }
      contextual = true;
      for (Node node : nodes) enqueue(node);
      changed = solveForward();
    } while (changed);
  }

  private void transferRequirement(Exprent expression, SemanticFacts required) {
    if (expression instanceof AssignmentExprent assignment) {
      if (assignment.getCondType() == null)
        require(assignment.getRight(), required);
      else if (isBitwise(assignment.getCondType())) {
        SemanticFacts flags = flags(required);
        require(assignment.getLeft(), flags);
        require(assignment.getRight(), flags);
      }
      return;
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      if (function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
        require(operands.get(1), required);
        require(operands.get(2), required);
      } else if (isValuePreservingCast(function)
        || isIntegralCast(function)
          && required.domains().stream().allMatch(
            domain -> analysis.mappings.fitsIntegralType(domain, primitiveDescriptor(function.getExprType())))) {
        require(operands.get(0), required);
      } else if (isBitwise(function)) {
        for (Exprent operand : operands) require(operand, flags(required));
      }
      for (String domain : required.domains()) {
        for (var field : analysis.mappings.bitFields(domain)) {
          if (field.selectorMask() != 0)
            continue;
          SemanticBitAccess.Packing packing = SemanticBitAccess.packing(expression, field.shift(), field.bits());
          if (packing != null)
            require(packing.value(), SemanticFacts.of(field.domain(), null));
        }
      }
      return;
    }
    if (expression instanceof NewExprent creation) {
      if (creation.getNewType().arrayDim > 0) {
        for (ArraySemantics shape : required.arrays()) {
          VarType elementType = creation.getNewType().decreaseArrayDim();
          for (int index = 0; index < creation.getLstArrayElements().size(); index++) {
            require(creation.getLstArrayElements().get(index),
              SemanticAnalysis.elementFacts(shape, analysis.slotElementDomain(shape, index), elementType));
          }
          for (SemanticHeap.Store store : heap.stores(creation)) {
            require(store.value(), analysis.project(shape, store.access().getIndex(), store.access().getExprType()));
          }
        }
      } else if (creation.getConstructor() != null) {
        for (ContainerSemantics shape : required.containers()) {
          for (InvocationExprent invocation : heap.containerUses(creation)) SemanticUses.containerUse(invocation, shape, this);
        }
        Exprent boxed = boxedArgument(creation.getConstructor());
        if (boxed != null)
          require(boxed, required);
      }
      return;
    }
    if (expression instanceof InvocationExprent invocation) {
      Exprent boxed = boxedArgument(invocation);
      if (boxed != null)
        require(boxed, required);
      else if (isUnboxing(invocation)
        && (unboxingPreservesStorage(invocation)
          || required.domains().stream().allMatch(
            domain -> analysis.mappings.fitsIntegralType(domain, primitiveDescriptor(invocation.getExprType()))))) {
        require(invocation.getInstance(), required);
      } else {
        Integer source = analysis.mappings.returnDomainSource(invocationKey(invocation));
        if (source != null && source >= 0 && source < invocation.getLstParameters().size())
          require(invocation.getLstParameters().get(source), required);
      }
    }
  }

  private SemanticFacts flags(SemanticFacts requested) {
    Set<String> domains = new HashSet<>();
    for (String domain : requested.domains())
      if ("flags".equals(analysis.mappings.domainKind(domain)))
        domains.add(domain);
    return new SemanticFacts(domains, Set.of(), Set.of(), Set.of(), false);
  }

  private void enqueue(Node node) {
    if (!node.queued) {
      node.queued = true;
      pending.addLast(node);
    }
  }
}
