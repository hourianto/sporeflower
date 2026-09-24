package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.struct.*;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.struct.gen.FieldDescriptor;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.NewClassNameBuilder;

import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/** Capture and apply declaration names while the context still uses original identities. */
public final class NamingPlanApplication {
  private NamingPlanApplication() { }

  public static NamingPlan capture(StructContext context, PoolInterceptor interceptor) {
    Map<String, String> classes = new LinkedHashMap<>();
    Map<Member, String> fields = new LinkedHashMap<>();
    Map<Member, String> methods = new LinkedHashMap<>();
    for (StructClass owner : context.getOwnClasses()) {
      String mapped = interceptor == null ? null : interceptor.getName(owner.qualifiedName);
      classes.put(owner.qualifiedName, mapped == null ? owner.qualifiedName : mapped);
      captureMembers(owner, owner.getFields(), fields, interceptor);
      captureMembers(owner, owner.getMethods(), methods, interceptor);
    }
    return new NamingPlan(classes, fields, methods);
  }

  private static void captureMembers(StructClass owner, Iterable<? extends StructMember> members,
                                     Map<Member, String> result, PoolInterceptor interceptor) {
    for (StructMember member : members) {
      Member key = member instanceof StructField field
        ? new Member(owner.qualifiedName, field.getName(), field.getDescriptor())
        : new Member(owner.qualifiedName, ((StructMethod)member).getName(), ((StructMethod)member).getDescriptor());
      String mapped = interceptor == null ? null : interceptor.getName(key.owner() + " " + key.name() + " " + key.descriptor());
      result.put(key, mapped == null ? key.name() : mapped.split(" ")[1]);
    }
  }

  public static void apply(NamingPlan plan, StructContext context, PoolInterceptor interceptor) {
    var classes = plan.classes();
    var fields = plan.fields();
    var methods = plan.methods();
    NamingPlan inventory = capture(context, null);
    if (!classes.keySet().equals(inventory.classes().keySet())
        || !fields.keySet().equals(inventory.fields().keySet()) || !methods.keySet().equals(inventory.methods().keySet())) {
      throw new IllegalArgumentException("Prepared mapping does not match the input declarations");
    }
    if (classes.values().stream().distinct().count() != classes.size()) {
      throw new IllegalArgumentException("Prepared mapping contains duplicate class names");
    }
    for (String emitted : classes.values()) {
      StructClass occupied = context.getClass(emitted);
      if (occupied != null && !occupied.isOwn()) throw new IllegalArgumentException("Prepared name collides with a library class: " + emitted);
    }
    NewClassNameBuilder types = name -> classes.getOrDefault(name, name);
    validateMembers(fields, types, false);
    validateMembers(methods, types, true);
    validateSourceNames(plan, context);
    plan.innerNames().forEach((original, simple) -> {
      if (!classes.containsKey(original)) throw new IllegalArgumentException("Unknown prepared nested class: " + original);
      interceptor.setInnerName(classes.get(original), simple);
    });
    classes.forEach((original, emitted) -> { if (!original.equals(emitted)) interceptor.addName(original, emitted); });
    applyMemberNames(plan, interceptor);
  }

  static void applyMemberNames(NamingPlan plan, PoolInterceptor interceptor) {
    var classes = plan.classes();
    NewClassNameBuilder types = name -> classes.getOrDefault(name, name);
    plan.fields().forEach((key, name) -> {
      if (!key.name().equals(name)) interceptor.addName(key.owner() + " " + key.name() + " " + key.descriptor(),
        classes.get(key.owner()) + " " + name + " " + descriptor(types, key.descriptor(), false));
    });
    plan.methods().forEach((key, name) -> {
      if (!key.name().equals(name)) interceptor.addName(key.owner() + " " + key.name() + " " + key.descriptor(),
        classes.get(key.owner()) + " " + name + " " + descriptor(types, key.descriptor(), true));
    });
  }

  private static void validateMembers(Map<Member, String> members, NewClassNameBuilder types, boolean method) {
    Set<Member> identities = new HashSet<>();
    for (var entry : members.entrySet()) {
      Member key = entry.getKey();
      String descriptor = descriptor(types, key.descriptor(), method);
      if (!identities.add(new Member(types.buildNewClassname(key.owner()), entry.getValue(), descriptor))) {
        throw new IllegalArgumentException("Prepared mapping contains duplicate member: " + key.owner() + "." + entry.getValue() + descriptor);
      }
    }
  }

  private static String descriptor(NewClassNameBuilder types, String descriptor, boolean method) {
    String mapped = method ? MethodDescriptor.parseDescriptor(descriptor).buildNewDescriptor(types)
      : FieldDescriptor.parseDescriptor(descriptor).buildNewDescriptor(types);
    return mapped == null ? descriptor : mapped;
  }

  private static void validateSourceNames(NamingPlan plan, StructContext context) {
    boolean removeSynthetic = DecompilerContext.getOption(IFernflowerPreferences.REMOVE_SYNTHETIC);
    for (StructClass owner : context.getOwnClasses()) {
      Set<String> fields = new HashSet<>();
      for (StructField field : owner.getFields()) {
        if (removeSynthetic && field.isSynthetic()) continue;
        String name = plan.fields().get(new Member(owner.qualifiedName, field.getName(), field.getDescriptor()));
        if (!fields.add(name)) throw new IllegalArgumentException("Prepared field names collide in Java: " + owner.qualifiedName + "." + name);
      }
      Set<String> methods = new HashSet<>();
      for (StructMethod method : owner.getMethods()) {
        Member key = new Member(owner.qualifiedName, method.getName(), method.getDescriptor());
        String name = plan.methods().get(key);
        if (method.getName().startsWith("<") && !name.equals(method.getName())) {
          throw new IllegalArgumentException("Prepared mapping renames an initializer: " + key);
        }
        StructMethod target = SourceMethodSemantics.forwardingBridgeTarget(context, owner, method);
        boolean retainedBridge = target != null && !name.equals(plan.methods().get(
          new Member(owner.qualifiedName, target.getName(), target.getDescriptor())));
        if (!retainedBridge && (removeSynthetic && method.isSynthetic() || method.hasModifier(CodeConstants.ACC_BRIDGE)
            && DecompilerContext.getOption(IFernflowerPreferences.REMOVE_BRIDGE))) continue;
        String signature = name + SourceMethodSemantics.parameterDescriptor(method.getDescriptor());
        if (!methods.add(signature)) throw new IllegalArgumentException("Prepared method names collide in Java: " + owner.qualifiedName + "." + signature);
      }
    }
    for (var entry : plan.parameters().entrySet()) {
      Member key = entry.getKey();
      StructClass owner = context.getClass(key.owner());
      StructMethod method = owner == null ? null : owner.getMethod(key.name(), key.descriptor());
      if (method == null) throw new IllegalArgumentException("Unknown prepared parameter owner: " + key);
      Set<Integer> slots = new HashSet<>();
      int slot = method.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
      for (var parameter : method.methodDescriptor().params) {
        slots.add(slot);
        slot += parameter.stackSize;
      }
      if (!slots.containsAll(entry.getValue().keySet())) throw new IllegalArgumentException("Invalid prepared parameter slot for " + key);
    }
  }
}
