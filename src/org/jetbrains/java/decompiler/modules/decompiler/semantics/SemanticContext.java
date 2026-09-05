package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.*;

/** Immutable definition keys and lexical guard facts captured before printable locals merge. */
final class SemanticContext {
  record Variable(int index, int version) {}
  record Key(Object operation, List<Key> operands) {}
  record Range(long min, long max) {
    static final Range INT = new Range(Integer.MIN_VALUE, Integer.MAX_VALUE);
    Range intersect(Range other) { return new Range(Math.max(min, other.min), Math.min(max, other.max)); }
  }
  private record Environment(Map<Key, Long> values, Map<Key, Range> ranges, Map<Key, Set<Long>> excluded) {
    static final Environment EMPTY = new Environment(Map.of(), Map.of(), Map.of());
  }
  private final Deque<SemanticLoopIndex> activeLoops = new ArrayDeque<>();
  private final Map<Exprent, SemanticLoopIndex> loopIndexes = new IdentityHashMap<>();
  private final Map<Exprent, Key> keys = new IdentityHashMap<>();
  private final Map<Exprent, Environment> environments = new IdentityHashMap<>();
  private final Map<Key, List<Exprent>> definitions = new HashMap<>();
  private final Set<Key> unknownDefinitions = new HashSet<>();
  private final Set<Key> activeRanges = new HashSet<>();
  private final Map<Exprent, Range> rangeCache = new IdentityHashMap<>();
  private boolean definitionsComplete;

  void markParameter(Key key) {
    // The incoming value is another definition, even if the parameter is
    // subsequently overwritten. It must not acquire a later assignment's facts.
    unknownDefinitions.add(key);
    rangeCache.clear();
  }

  List<Exprent> definitions(Exprent expression) {
    Key key = key(expression);
    return expression instanceof VarExprent && !unknownDefinitions.contains(key) ? definitions.get(key) : null;
  }

  static Key variable(int index, int version) { return new Key(new Variable(index, version), List.of()); }
  static Key constant(long value) { return new Key(value, List.of()); }
  static Key operation(FunctionExprent.FunctionType type, Key... operands) {
    if (Arrays.stream(operands).anyMatch(Objects::isNull)) return null;
    if (type == FunctionExprent.FunctionType.AND && operands.length == 2 && operands[0].operation() instanceof Long
        && !(operands[1].operation() instanceof Long)) return new Key(type, List.of(operands[1], operands[0]));
    return new Key(type, List.of(operands));
  }

  Key key(Exprent expression) { return keys.get(expression); }

  Long known(Exprent at, Key key) {
    if (key == null) return null;
    if (key.operation() instanceof Long value) return value;
    Environment environment = environments.getOrDefault(at, Environment.EMPTY);
    Long value = environment.values().get(key);
    if (value != null) return value;
    if (key.operation() == FunctionExprent.FunctionType.AND && key.operands().get(1).operation() instanceof Long mask) {
      Long whole = known(at, key.operands().get(0));
      if (whole != null) return whole & mask;
      // A single-bit mask has exactly two possible values, including for a
      // signed long's top bit. Excluding one establishes the other.
      if (Long.bitCount(mask) == 1) {
        Set<Long> excluded = environment.excluded().getOrDefault(key, Set.of());
        if (excluded.contains(0L)) return mask;
        if (excluded.contains(mask)) return 0L;
      }
    }
    return null;
  }

  boolean excludes(Exprent at, Key key, long value) {
    if (key == null) return false;
    Long known = known(at, key);
    if (known != null) return known != value;
    Environment environment = environments.getOrDefault(at, Environment.EMPTY);
    Range range = environment.ranges().get(key);
    return environment.excluded().getOrDefault(key, Set.of()).contains(value)
      || range != null && (value < range.min() || value > range.max());
  }

  Long value(Exprent expression) {
    Long constant = integral(expression);
    if (constant != null) return constant;
    Long known = known(expression, key(expression));
    if (known != null) return known;
    Range range = range(expression);
    return range.min() == range.max() ? range.min() : null;
  }

  Range range(Exprent expression) {
    Range cached = rangeCache.get(expression);
    if (cached != null) return cached;
    Range intrinsic = intrinsicRange(expression);
    Key key = key(expression);
    Range guard = key == null ? null : environments.getOrDefault(expression, Environment.EMPTY).ranges().get(key);
    Range result = guard == null ? intrinsic : intrinsic.intersect(guard);
    if (definitionsComplete) rangeCache.put(expression, result);
    return result;
  }

  Integer loopResidue(Exprent index, int stride) {
    SemanticLoopIndex.Offset offset = SemanticLoopIndex.offset(index);
    if (offset == null) return null;
    SemanticLoopIndex loop = loopIndexes.get(offset.variable());
    if (loop == null || loop.step() % stride != 0 || !loop.noWrap() && Integer.bitCount(stride) != 1) return null;
    // This is called only for the whole array index. With a nonnegative,
    // nonwrapping counter, overflow in counter+constant produces a negative
    // index and cannot complete an array access. Do not apply that reasoning
    // recursively inside arithmetic which could wrap it back to positive.
    return Math.floorMod(loop.start() + offset.delta(), stride);
  }

  private Range intrinsicRange(Exprent expression) {
    Long value = integral(expression);
    if (value != null) return new Range(value, value);
    SemanticLoopIndex loop = loopIndexes.get(expression);
    if (loop != null && loop.noWrap()) return new Range(loop.start(), loop.maximum());
    List<Exprent> sources = definitionsComplete ? definitions(expression) : null;
    if (sources != null && !sources.isEmpty() && activeRanges.add(key(expression))) {
      try {
        long min = Long.MAX_VALUE, max = Long.MIN_VALUE;
        for (Exprent source : sources) {
          Range range = range(source);
          min = Math.min(min, range.min()); max = Math.max(max, range.max());
        }
        return new Range(min, max).intersect(typeRange(expression));
      } finally {
        activeRanges.remove(key(expression));
      }
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      Range left = range(operands.get(0));
      if (function.getFuncType() == FunctionExprent.FunctionType.I2B) return new Range(-128, 127);
      if (function.getFuncType() == FunctionExprent.FunctionType.I2S) return new Range(-32768, 32767);
      if (function.getFuncType() == FunctionExprent.FunctionType.I2C) return new Range(0, 65535);
      if (operands.size() == 2 && function.getExprType().type != VarType.VARTYPE_LONG.type) {
        Range right = range(operands.get(1));
        Long number = integral(operands.get(1));
        if (function.getFuncType() == FunctionExprent.FunctionType.AND) {
          Long mask = number == null ? integral(operands.get(0)) : number;
          if (mask != null && mask >= 0 && mask <= Integer.MAX_VALUE) return new Range(0, mask);
        }
        if (number != null && number > 0 && function.getFuncType() == FunctionExprent.FunctionType.REM) {
          return new Range(left.min() >= 0 ? 0 : 1 - number, number - 1);
        }
        if (number != null && function.getFuncType() == FunctionExprent.FunctionType.USHR) {
          int shift = number.intValue() & 31;
          if (shift != 0) return new Range(0, 0xffffffffL >>> shift);
        }
        if (function.getFuncType() == FunctionExprent.FunctionType.SHL
            && (number == null || (number.intValue() & 31) == 31)) return Range.INT;
        Range arithmetic = arithmeticRange(function.getFuncType(), left, right);
        if (arithmetic != null) return arithmetic;
      }
    }
    return typeRange(expression);
  }

  /** Returns null unless this arithmetic stays within int range without wrapping. */
  static Range arithmeticRange(FunctionExprent.FunctionType operation, Range left, Range right) {
    try {
      long min, max;
      switch (operation) {
        case ADD -> { min = Math.addExact(left.min(), right.min()); max = Math.addExact(left.max(), right.max()); }
        case SUB -> { min = Math.subtractExact(left.min(), right.max()); max = Math.subtractExact(left.max(), right.min()); }
        case MUL -> {
          long[] products = {Math.multiplyExact(left.min(), right.min()), Math.multiplyExact(left.min(), right.max()),
            Math.multiplyExact(left.max(), right.min()), Math.multiplyExact(left.max(), right.max())};
          min = Arrays.stream(products).min().orElseThrow(); max = Arrays.stream(products).max().orElseThrow();
        }
        case SHL -> {
          if (right.min() != right.max()) return null;
          long factor = 1L << ((int)right.min() & 31);
          min = Math.multiplyExact(left.min(), factor); max = Math.multiplyExact(left.max(), factor);
        }
        default -> { return null; }
      }
      // Crossing int overflow destroys the ordered interval. Both index bounds
      // and non-power-of-two record residues must use this same proof.
      return min >= Integer.MIN_VALUE && max <= Integer.MAX_VALUE ? new Range(min, max) : null;
    } catch (ArithmeticException ignored) {
      return null;
    }
  }

  private static Range typeRange(Exprent expression) {
    if (expression.getExprType().arrayDim != 0) return Range.INT;
    return switch (expression.getExprType().type) {
      case BYTE -> new Range(-128, 127);
      case SHORT -> new Range(-32768, 32767);
      case CHAR -> new Range(0, 65535);
      case LONG -> new Range(Long.MIN_VALUE, Long.MAX_VALUE);
      default -> Range.INT;
    };
  }

  void analyze(Statement root) {
    visit(root, Environment.EMPTY);
    definitionsComplete = true;
  }

  private void visit(Statement statement, Environment environment) {
    List<Exprent> own = statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents();
    for (Exprent expression : own) register(expression, environment);
    SemanticLoopIndex induction = statement instanceof DoStatement loop ? SemanticLoopIndex.analyze(loop, this) : null;
    Environment next = environment;
    for (Statement child : statement.getStats()) {
      Environment branch = statement instanceof SequenceStatement ? next : environment;
      if (statement instanceof IfStatement conditional) {
        if (child == conditional.getIfstat()) branch = refine(environment, conditional.getHeadexprent().getCondition(), true);
        else if (child == conditional.getElsestat()) branch = refine(environment, conditional.getHeadexprent().getCondition(), false);
      } else if (statement instanceof SwitchStatement selection) {
        int index = selection.getCaseStatements().indexOf(child);
        if (index >= 0) {
          List<Exprent> labels = selection.getCaseValues().get(index);
          // Fallthrough admits values belonging to earlier labels. Refine only
          // a single-label case whose incoming edges all originate at the head.
          if (labels.size() == 1 && child.getPredecessorEdges(StatEdge.TYPE_REGULAR).stream()
              .allMatch(edge -> edge.getSource() == selection.getFirst())) {
            Key selector = makeKey(((SwitchHeadExprent)selection.getHeadexprent()).getValue());
            Long value = integral(labels.get(0));
            if (value != null) branch = withValue(environment, selector, value);
            else if (labels.get(0) == null) {
              for (List<Exprent> cases : selection.getCaseValues()) for (Exprent label : cases) {
                Long excluded = integral(label);
                if (excluded != null) branch = withoutValue(branch, selector, excluded);
              }
            }
          }
        }
      }
      if (induction != null) activeLoops.push(induction);
      visit(child, withoutWrites(branch, child));
      if (induction != null) activeLoops.pop();
      if (statement instanceof SequenceStatement) {
        next = withoutWrites(next, child);
        // A returning branch cannot reach the following statement. Retain its
        // complementary guard without treating an ordinary fallthrough as an exit.
        if (child instanceof IfStatement conditional && conditional.getElsestat() == null && exits(conditional.getIfstat())) {
          next = refine(next, conditional.getHeadexprent().getCondition(), false);
        }
      }
    }
  }

  private static boolean exits(Statement statement) {
    if (statement == null) return false;
    List<Exprent> expressions = statement.getExprents();
    if (expressions != null) return !expressions.isEmpty() && expressions.get(expressions.size() - 1) instanceof ExitExprent;
    if (statement instanceof SequenceStatement && !statement.getStats().isEmpty()) {
      return exits(statement.getStats().get(statement.getStats().size() - 1));
    }
    if (statement instanceof IfStatement conditional) return exits(conditional.getIfstat()) && exits(conditional.getElsestat());
    return false;
  }

  private void register(Exprent expression, Environment environment) {
    Key key = makeKey(expression);
    if (key != null) keys.put(expression, key);
    environments.put(expression, environment);
    if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent) {
      Key target = makeKey(assignment.getLeft());
      if (assignment.getCondType() == null) definitions.computeIfAbsent(target, ignored -> new ArrayList<>()).add(assignment.getRight());
      else unknownDefinitions.add(target);
    } else if (expression instanceof FunctionExprent function && switch (function.getFuncType()) {
      case IPP, PPI, IMM, MMI -> true; default -> false;
    }) unknownDefinitions.add(makeKey(function.getLstOperands().get(0)));
    if (expression instanceof VarExprent) {
      for (SemanticLoopIndex loop : activeLoops) if (loop.variable().equals(key)) { loopIndexes.put(expression, loop); break; }
    }
    if (expression instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      List<Exprent> operands = function.getLstOperands();
      register(operands.get(0), environment);
      register(operands.get(1), refine(environment, operands.get(0), true));
      register(operands.get(2), refine(environment, operands.get(0), false));
    } else if (expression instanceof FunctionExprent function && (function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND
        || function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_OR)) {
      register(function.getLstOperands().get(0), environment);
      register(function.getLstOperands().get(1), refine(environment, function.getLstOperands().get(0),
        function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND));
    } else for (Exprent child : expression.getAllExprents()) register(child, environment);
  }

  private Key makeKey(Exprent expression) {
    if (keys.containsKey(expression)) return keys.get(expression);
    Key result = computeKey(expression);
    keys.put(expression, result);
    return result;
  }

  private Key computeKey(Exprent expression) {
    if (expression instanceof VarExprent variable) return variable(variable.getIndex(), variable.getVersion());
    Long value = integral(expression);
    if (value != null) return constant(value);
    if (expression instanceof FunctionExprent function && switch (function.getFuncType()) {
      case AND, OR, XOR, SHL, SHR, USHR, ADD, SUB, MUL, I2B, I2S, I2C, I2L, L2I -> true;
      default -> false;
    }) {
      List<Key> operands = function.getLstOperands().stream().map(this::makeKey).toList();
      if (operands.stream().noneMatch(Objects::isNull)) return operation(function.getFuncType(), operands.toArray(Key[]::new));
    }
    // Mutable fields and arbitrary invocations are not stable guard keys.
    return null;
  }

  private Environment refine(Environment environment, Exprent condition, boolean truth) {
    if (!(condition instanceof FunctionExprent function)) return environment;
    List<Exprent> operands = function.getLstOperands();
    if (function.getFuncType() == FunctionExprent.FunctionType.BOOL_NOT) return refine(environment, operands.get(0), !truth);
    if (truth && function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND
        || !truth && function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_OR) {
      return refine(refine(environment, operands.get(0), truth), operands.get(1), truth);
    }
    if (operands.size() != 2) return environment;
    Long value = integral(operands.get(1));
    Exprent compared = operands.get(0);
    Key key = makeKey(compared);
    FunctionExprent.FunctionType type = function.getFuncType();
    if (value == null) {
      value = integral(operands.get(0)); compared = operands.get(1); key = makeKey(compared);
      type = switch (type) { case LT -> FunctionExprent.FunctionType.GT; case LE -> FunctionExprent.FunctionType.GE;
        case GT -> FunctionExprent.FunctionType.LT; case GE -> FunctionExprent.FunctionType.LE; default -> type; };
    }
    if (value == null || key == null) return environment;
    if (type == FunctionExprent.FunctionType.EQ && truth || type == FunctionExprent.FunctionType.NE && !truth) return withValue(environment, key, value);
    if (type == FunctionExprent.FunctionType.NE && truth || type == FunctionExprent.FunctionType.EQ && !truth) {
      return withoutValue(environment, key, value);
    }
    if (!truth) type = switch (type) { case LT -> FunctionExprent.FunctionType.GE; case LE -> FunctionExprent.FunctionType.GT;
      case GT -> FunctionExprent.FunctionType.LE; case GE -> FunctionExprent.FunctionType.LT; default -> type; };
    // Ordered range analysis here is int-only. Long equality/exclusion facts
    // above remain usable, but truncating a long interval would prove false cases.
    if (compared.getExprType().equals(VarType.VARTYPE_LONG)
        || value <= Integer.MIN_VALUE || value >= Integer.MAX_VALUE) return environment;
    Range bound = switch (type) {
      case LT -> new Range(Integer.MIN_VALUE, value - 1); case LE -> new Range(Integer.MIN_VALUE, value);
      case GT -> new Range(value + 1, Integer.MAX_VALUE); case GE -> new Range(value, Integer.MAX_VALUE);
      default -> null;
    };
    if (bound == null) return environment;
    Map<Key, Range> ranges = new HashMap<>(environment.ranges());
    ranges.merge(key, bound, Range::intersect);
    return new Environment(environment.values(), Map.copyOf(ranges), environment.excluded());
  }

  private static Environment withoutValue(Environment environment, Key key, long value) {
    if (key == null) return environment;
    Map<Key, Set<Long>> excluded = new HashMap<>(environment.excluded());
    Set<Long> values = new HashSet<>(excluded.getOrDefault(key, Set.of())); values.add(value);
    excluded.put(key, Set.copyOf(values));
    return new Environment(environment.values(), environment.ranges(), Map.copyOf(excluded));
  }

  private static Environment withValue(Environment environment, Key key, long value) {
    if (key == null) return environment;
    Map<Key, Long> values = new HashMap<>(environment.values()); values.put(key, value);
    Map<Key, Range> ranges = new HashMap<>(environment.ranges()); ranges.put(key, new Range(value, value));
    return new Environment(Map.copyOf(values), Map.copyOf(ranges), environment.excluded());
  }

  private Environment withoutWrites(Environment environment, Statement statement) {
    if (environment.values().isEmpty() && environment.ranges().isEmpty() && environment.excluded().isEmpty()) return environment;
    Set<Key> written = new HashSet<>(); collectWrites(statement, written);
    if (written.isEmpty()) return environment;
    Map<Key, Long> values = new HashMap<>(environment.values());
    Map<Key, Range> ranges = new HashMap<>(environment.ranges());
    Map<Key, Set<Long>> excluded = new HashMap<>(environment.excluded());
    values.keySet().removeIf(key -> dependsOn(key, written)); ranges.keySet().removeIf(key -> dependsOn(key, written));
    excluded.keySet().removeIf(key -> dependsOn(key, written));
    return new Environment(Map.copyOf(values), Map.copyOf(ranges), Map.copyOf(excluded));
  }

  private static boolean dependsOn(Key key, Set<Key> written) {
    return written.contains(key) || key.operands().stream().anyMatch(child -> dependsOn(child, written));
  }

  private void collectWrites(Statement statement, Set<Key> written) {
    List<Exprent> own = statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents();
    for (Exprent expression : own) collectWrites(expression, written);
    for (Statement child : statement.getStats()) collectWrites(child, written);
  }

  private void collectWrites(Exprent expression, Set<Key> written) {
    if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent) written.add(makeKey(assignment.getLeft()));
    if (expression instanceof FunctionExprent function && switch (function.getFuncType()) {
      case IPP, PPI, IMM, MMI -> true; default -> false;
    }) written.add(makeKey(function.getLstOperands().get(0)));
    for (Exprent child : expression.getAllExprents()) collectWrites(child, written);
  }

  static Long integral(Exprent expression) {
    if (expression instanceof ConstExprent constant && constant.getValue() instanceof Number value
        && !(value instanceof Float) && !(value instanceof Double)) return value.longValue();
    return null;
  }
}
