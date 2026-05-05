package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class RenameMembersConflictRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testRenameMembersConflictResolutionKeepsRunOverloadsAndValidConstructorSyntax() throws IOException {
    Path hasCoords = writeSource("ren/HasCoords.java", """
package ren;

public interface HasCoords {
  int n = 1;
  int o = 2;
}
""");

    Path baseUnit = writeSource("ren/BaseUnit.java", """
package ren;

public abstract class BaseUnit {
  protected int n;
  protected int o;
  protected int p;
}
""");

    Path unitRunner = writeSource("ren/UnitRunner.java", """
package ren;

public class UnitRunner extends BaseUnit implements HasCoords, Runnable {
  public UnitRunner(int x, int y) {
    super.n = x;
    super.o = y;
    this.p = x + y;
  }

  @Override
  public void run() {
    this.p++;
  }

  public int a() {
    return super.n + super.o;
  }

  public int a(int delta) {
    return this.p + delta;
  }
}
""");

    Path factory = writeSource("ren/Factory.java", """
package ren;

public final class Factory {
  private Factory() {
  }

  public static UnitRunner create() {
    return new UnitRunner(1, 2);
  }
}
""");

    compileJava8NoDebug(List.of(hasCoords, baseUnit, unitRunner, factory), outRoot());

    String ifaceContent = decompileDirectory(outRoot(), "ren/HasCoords.java");
    String unitContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("ren/UnitRunner.java"));
    String factoryContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("ren/Factory.java"));

    assertTrue(unitContent.contains("public void run()"), unitContent);
    assertFalse(unitContent.contains("renamed from: run () void"), unitContent);
    assertTrue(unitContent.contains("public int a()"), unitContent);
    assertTrue(Pattern.compile("public int a\\(int").matcher(unitContent).find(), unitContent);
    assertFalse(Pattern.compile("public int method_\\d+\\(int").matcher(unitContent).find(), unitContent);

    assertTrue(ifaceContent.contains("int field_0 = 1;"), ifaceContent);
    assertTrue(ifaceContent.contains("int field_1 = 2;"), ifaceContent);
    assertFalse(ifaceContent.contains("int n = 1;"), ifaceContent);
    assertFalse(ifaceContent.contains("int o = 2;"), ifaceContent);

    assertTrue(factoryContent.contains("new UnitRunner(1, 2)"), factoryContent);
    assertFalse(Pattern.compile("new\\s+UnitRunner\\s*;").matcher(factoryContent).find(), factoryContent);

    recompile();
  }

  @Test
  public void testRawRenameSplitsInheritedInterfaceReturnOnlyCollision() throws IOException {
    Path base = writeSource("raw/Base.java", """
package raw;

public class Base {
  public boolean b() {
    return true;
  }

  public int baseValue() {
    return b() ? 1 : 0;
  }
}
""");

    Path iface = writeSource("raw/HasArray.java", """
package raw;

public interface HasArray {
  int[] c();
}
""");

    Path child = writeSource("raw/Child.java", """
package raw;

public class Child extends Base implements HasArray {
  public int[] c() {
    return new int[]{1, 2, 3};
  }

  public int childValue() {
    return c().length;
  }
}
""");

    Path user = writeSource("raw/User.java", """
package raw;

public class User {
  public static int use(HasArray array, Child child) {
    return array.c().length + child.c().length + child.baseValue();
  }
}
""");

    compileJava8NoDebug(List.of(base, iface, child, user), outRoot());
    renameUtf8Constant(outRoot().resolve("raw/HasArray.class"), 'c', 'b');
    renameUtf8Constant(outRoot().resolve("raw/Child.class"), 'c', 'b');
    renameUtf8Constant(outRoot().resolve("raw/User.class"), 'c', 'b');

    String childContent = decompileDirectory(outRoot(), "raw/Child.java");
    String baseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("raw/Base.java"));
    String ifaceContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("raw/HasArray.java"));
    String userContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("raw/User.java"));

    assertTrue(
      Pattern.compile("boolean\\s+method_\\d+\\s*\\(").matcher(baseContent).find()
        || Pattern.compile("int\\[\\]\\s+method_\\d+\\s*\\(").matcher(ifaceContent).find()
        || Pattern.compile("public\\s+int\\[\\]\\s+method_\\d+\\s*\\(").matcher(childContent).find(),
      baseContent + "\n" + ifaceContent + "\n" + childContent + "\n" + userContent
    );

    recompile();
  }
}
