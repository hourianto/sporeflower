package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.ClassesProcessor;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.decompiler.PrintStreamLogger;
import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.junit.jupiter.api.Test;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.HashMap;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertSame;

public class StructContextRemappedAliasTest {
  @Test
  public void testRemappedClassKeepsOriginalLookupAlias() {
    CountingContextSource source = new CountingContextSource();
    StructContext structContext = new StructContext(null, null);
    PoolInterceptor interceptor = new PoolInterceptor();
    interceptor.addName("a", "defpackage/a");

    DecompilerContext.setCurrentContext(new DecompilerContext(
      new HashMap<>(),
      new PrintStreamLogger(System.out),
      structContext,
      new ClassesProcessor(structContext),
      interceptor
    ));

    structContext.addSpace(source, true);
    assertEquals(1, source.classByteReads);

    StructClass oldNameLookup = structContext.getClass("a");
    assertEquals("defpackage/a", oldNameLookup.qualifiedName);
    assertEquals(1, source.classByteReads);

    assertSame(oldNameLookup, structContext.getClass("a"));
    assertSame(oldNameLookup, structContext.getClass("defpackage/a"));
    assertEquals(1, source.classByteReads);
  }

  private static final class CountingContextSource implements IContextSource {
    private int classByteReads;

    @Override
    public String getName() {
      return "counting";
    }

    @Override
    public Entries getEntries() {
      return new Entries(List.of(Entry.atBase("a")), List.of(), List.of());
    }

    @Override
    public byte[] getClassBytes(String className) {
      if (!"a".equals(className)) {
        return null;
      }

      classByteReads++;
      return MINIMAL_CLASS_A.clone();
    }

    @Override
    public InputStream getInputStream(String resource) throws IOException {
      if ("a.class".equals(resource)) {
        return new ByteArrayInputStream(getClassBytes("a"));
      }

      return null;
    }
  }

  private static final byte[] MINIMAL_CLASS_A = {
    (byte)0xCA, (byte)0xFE, (byte)0xBA, (byte)0xBE,
    0x00, 0x00, 0x00, 0x34,
    0x00, 0x05,
    0x07, 0x00, 0x02,
    0x01, 0x00, 0x01, 0x61,
    0x07, 0x00, 0x04,
    0x01, 0x00, 0x10, 0x6A, 0x61, 0x76, 0x61, 0x2F,
    0x6C, 0x61, 0x6E, 0x67, 0x2F, 0x4F, 0x62, 0x6A,
    0x65, 0x63, 0x74,
    0x00, 0x21,
    0x00, 0x01,
    0x00, 0x03,
    0x00, 0x00,
    0x00, 0x00,
    0x00, 0x00,
    0x00, 0x00
  };
}
