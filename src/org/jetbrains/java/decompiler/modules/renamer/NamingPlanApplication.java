package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.struct.*;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.struct.gen.FieldDescriptor;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.NewClassNameBuilder;
import org.jetbrains.java.decompiler.util.TextUtil;

import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/** Inventory, validate and apply names while the context still uses original identities. */
public final class NamingPlanApplication {
  private NamingPlanApplication() { }

  public static NamingPlan inventory(StructContext context) {
    Map<String, String> classes = new LinkedHashMap<>();
    Map<Member, String> fields = new LinkedHashMap<>();
    Map<Member, String> methods = new LinkedHashMap<>();
    for (StructClass owner : context.getOwnClasses()) {
      classes.put(owner.qualifiedName, owner.qualifiedName);
      inventoryMembers(owner, owner.getFields(), fields);
      inventoryMembers(owner, owner.getMethods(), methods);
    }
    return new NamingPlan(classes, fields, methods);
  }

  private static void inventoryMembers(StructClass owner, Iterable<? extends StructMember> members,
                                     Map<Member, String> result) {
    for (StructMember member : members) {
      Member key = member instanceof StructField field
        ? new Member(owner.qualifiedName, field.getName(), field.getDescriptor())
        : new Member(owner.qualifiedName, ((StructMethod)member).getName(), ((StructMethod)member).getDescriptor());
      result.put(key, key.name());
    }
  }

  public static void validate(NamingPlan plan, StructContext context) {
    PoolInterceptor interceptor = new PoolInterceptor();
    interceptor.bindMemberReferences(context);
    validate(plan, context, interceptor);
  }

  public static void validate(NamingPlan plan, StructContext context, PoolInterceptor interceptor) {
    var classes = plan.classes();
    var fields = plan.fields();
    var methods = plan.methods();
    NamingPlan inventory = inventory(context);
    if (!classes.keySet().equals(inventory.classes().keySet())
        || !fields.keySet().equals(inventory.fields().keySet()) || !methods.keySet().equals(inventory.methods().keySet())) {
      throw new IllegalArgumentException("Prepared mapping does not match the input declarations");
    }
    // Discover lazy library ancestors before checking package groups, as allocation does.
    var order = NamingHierarchy.order(context);
    ClassNameAllocator.validate(plan, context);
    validateSourceNames(plan, context);
    MethodNameAllocator.validate(plan, context, order, interceptor);
    FieldNameAllocator.validate(plan, context, interceptor);
  }

  public static void apply(NamingPlan plan, PoolInterceptor interceptor) {
    var classes = plan.classes();
    plan.innerNames().forEach((original, simple) -> interceptor.setInnerName(classes.get(original), simple));
    classes.forEach((original, emitted) -> { if (!original.equals(emitted)) interceptor.addName(original, emitted); });
    applyMemberNames(plan, interceptor);
  }

  private static void applyMemberNames(NamingPlan plan, PoolInterceptor interceptor) {
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

  private static String descriptor(NewClassNameBuilder types, String descriptor, boolean method) {
    String mapped = method ? MethodDescriptor.parseDescriptor(descriptor).buildNewDescriptor(types)
      : FieldDescriptor.parseDescriptor(descriptor).buildNewDescriptor(types);
    return mapped == null ? descriptor : mapped;
  }

  private static void validateSourceNames(NamingPlan plan, StructContext context) {
    for (StructClass owner : context.getOwnClasses()) {
      for (StructField field : owner.getFields()) {
        Member key = new Member(owner.qualifiedName, field.getName(), field.getDescriptor());
        String name = plan.fields().get(key);
        if (ConverterHelper.mustRenameForJava(IIdentifierRenamer.Type.ELEMENT_FIELD, name)) {
          throw new IllegalArgumentException("Invalid prepared field name: " + owner.qualifiedName + "." + name);
        }
      }
      for (StructMethod method : owner.getMethods()) {
        Member key = new Member(owner.qualifiedName, method.getName(), method.getDescriptor());
        String name = plan.methods().get(key);
        if (method.getName().startsWith("<") && !name.equals(method.getName())) {
          throw new IllegalArgumentException("Prepared mapping renames an initializer: " + key);
        }
      }
    }
    for (var entry : plan.parameters().entrySet()) {
      Member key = entry.getKey();
      StructClass owner = context.getClass(key.owner());
      StructMethod method = owner == null ? null : owner.getMethod(key.name(), key.descriptor());
      if (method == null) throw new IllegalArgumentException("Unknown prepared parameter owner: " + key);
      Set<Integer> slots = parameterSlots(method);
      if (!slots.containsAll(entry.getValue().keySet())) throw new IllegalArgumentException("Invalid prepared parameter slot for " + key);
      Set<String> names = new HashSet<>();
      for (String name : entry.getValue().values()) {
        if (!TextUtil.isValidIdentifier(name, method.getBytecodeVersion(), method) || !names.add(name)) {
          throw new IllegalArgumentException("Invalid or duplicate prepared parameter name for " + key + ": " + name);
        }
      }
    }
  }

  static Set<Integer> parameterSlots(StructMethod method) {
    Set<Integer> slots = new HashSet<>();
    int slot = method.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    for (var parameter : method.methodDescriptor().params) {
      slots.add(slot);
      slot += parameter.stackSize;
    }
    return slots;
  }
}
