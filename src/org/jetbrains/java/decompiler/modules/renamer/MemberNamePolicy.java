package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer.Type;

/** Requests member spellings independently of the scopes in which they must fit. */
final class MemberNamePolicy {
  private final IIdentifierRenamer helper;
  private final ConverterHelper fallback;

  MemberNamePolicy(IIdentifierRenamer helper, ConverterHelper fallback) {
    this.helper = helper;
    this.fallback = fallback;
  }

  boolean shouldRename(Type type, Member member) {
    return helper.toBeRenamed(type, member.owner(), member.name(), member.descriptor())
      || ConverterHelper.mustRenameForJava(type, member.name());
  }

  String mappedName(Type type, Member member) {
    if (!(helper instanceof Tiny2IdentifierRenamer tiny)) return null;
    return type == Type.ELEMENT_METHOD
      ? tiny.getMappedMethodName(member.owner(), member.name(), member.descriptor())
      : tiny.getMappedFieldName(member.owner(), member.name(), member.descriptor());
  }

  Choice choose(Type type, Member member) {
    String requested = null;
    if (!shouldRename(type, member)) {
      requested = member.name();
    } else if (helper instanceof Tiny2IdentifierRenamer) {
      requested = mappedName(type, member);
    } else if (helper.getClass() != ConverterHelper.class) {
      requested = generate(helper, type, member);
    }
    return new Choice(type, member, validName(type, requested));
  }

  private static String validName(Type type, String name) {
    return ConverterHelper.mustRenameForJava(type, name) || name.startsWith("<") ? null : name;
  }

  final class Choice {
    private final Type type;
    private final Member member;
    private final String requested;

    private Choice(Type type, Member member, String requested) {
      this.type = type;
      this.member = member;
      this.requested = requested;
    }

    String requestedName() { return requested; }

    String nextName() {
      return generate(fallback, type, member);
    }
  }

  private static String generate(IIdentifierRenamer renamer, Type type, Member member) {
    return type == Type.ELEMENT_METHOD
      ? renamer.getNextMethodName(member.owner(), member.name(), member.descriptor())
      : renamer.getNextFieldName(member.owner(), member.name(), member.descriptor());
  }
}
