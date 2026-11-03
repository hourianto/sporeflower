// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;

import java.io.IOException;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Builds a complete naming plan in original identities, then applies it once. */
public class IdentifierConverter {
  private final StructContext context;
  private final IIdentifierRenamer helper;
  private final PoolInterceptor interceptor;

  public IdentifierConverter(StructContext context, IIdentifierRenamer helper, PoolInterceptor interceptor) {
    this.context = context;
    this.helper = helper;
    this.interceptor = interceptor;
  }

  public NamingPlan rename(NamingPlan prepared) {
    try {
      interceptor.bindMemberReferences(context);
      NamingPlan plan = prepared;
      if (plan == null) {
        plan = allocate();
      } else {
        if (!(helper instanceof Tiny2IdentifierRenamer)) {
          throw new IllegalArgumentException("Prepared names require a complete Tiny mapping");
        }
        NamingPlanApplication.validate(plan, context, interceptor);
      }
      // No provisional names reach constant pools: class placement, method
      // contracts and field qualifiers all resolve against original identities.
      NamingPlanApplication.apply(plan, interceptor);
      if (helper instanceof Tiny2IdentifierRenamer tiny) tiny.bindParameterNames(plan);
      retainRenamedBridges(plan);
      context.reloadContext(plan.classes());
      return plan;
    } catch (IOException ex) {
      throw new RuntimeException("Renaming failed with exception!", ex);
    }
  }

  private NamingPlan allocate() {
    List<StructClass> order = NamingHierarchy.order(context);
    ConverterHelper fallback = new ConverterHelper();
    MemberNamePolicy policy = new MemberNamePolicy(helper, fallback);
    ClassNameAllocator classes = new ClassNameAllocator(context, helper, fallback, order);
    classes.allocate();
    NamingPlan inventory = NamingPlanApplication.inventory(context);
    Map<Member, String> methods = new LinkedHashMap<>(inventory.methods());
    Map<Member, String> fields = new LinkedHashMap<>(inventory.fields());
    MethodNameAllocator.allocate(context, policy, order, interceptor, methods);
    FieldNameAllocator.allocate(context, policy, order, classes, interceptor, fields);
    return new NamingPlan(classes.classes(), fields, methods,
      helper instanceof Tiny2IdentifierRenamer tiny ? tiny.completedParameterNames(context) : Map.of(), classes.innerNames());
  }

  private void retainRenamedBridges(NamingPlan plan) {
    for (StructClass owner : context.getOwnClasses()) {
      for (StructMethod bridge : owner.getMethods()) {
        StructMethod target = SourceMethodSemantics.forwardingBridgeTarget(context, owner, bridge);
        if (target == null) continue;
        String bridgeName = plan.methods().get(new Member(owner.qualifiedName, bridge.getName(), bridge.getDescriptor()));
        if (!bridgeName.equals(plan.methods().get(new Member(owner.qualifiedName, target.getName(), target.getDescriptor())))) {
          String descriptor = MethodDescriptor.parseDescriptor(bridge.getDescriptor()).buildNewDescriptor(plan.classes()::get);
          interceptor.retainBridge(plan.classes().get(owner.qualifiedName), bridgeName,
            descriptor == null ? bridge.getDescriptor() : descriptor);
        }
      }
    }
  }
}
