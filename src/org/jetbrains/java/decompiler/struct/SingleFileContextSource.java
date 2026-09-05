// Copyright 2000-2022 JetBrains s.r.o. and ForgeFlower contributors Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.struct;

import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.main.extern.IResultSaver;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.util.List;

class SingleFileContextSource implements IContextSource {
  private final File file;
  private final String qualifiedName;
  private final byte[] contents;

  public SingleFileContextSource(final File singleFile) throws IOException {
    this.file = singleFile;
    if (!singleFile.isFile()) {
      this.contents = null;
      this.qualifiedName = null;
    } else {
      this.contents = InterpreterUtil.getBytes(singleFile);

      if (this.contents != null && singleFile.getName().endsWith(CLASS_SUFFIX) && ClassFileMagic.isClassFile(this.contents)) {
        try (final DataInputFullStream is = new DataInputFullStream(this.contents)) {
          var clazz = StructClass.create(is, false);
          this.qualifiedName = clazz.qualifiedName;
        }
      } else {
        this.qualifiedName = null;
      }
    }
  }

  @Override
  public String getName() {
    return "file " + this.file;
  }

  @Override
  public Entries getEntries() {
    if (this.contents == null) {
      return Entries.EMPTY;
    } else if (this.qualifiedName != null) {
      return new Entries(List.of(Entry.atBase(this.qualifiedName)), List.of(), List.of());
    } else {
      return new Entries(List.of(), List.of(), List.of(Entry.atBase(this.file.getName())));
    }
  }

  @Override
  public InputStream getInputStream(String resource) throws IOException {
    return new ByteArrayInputStream(this.contents);
  }

  @Override
  public IOutputSink createOutputSink(IResultSaver saver) {
    return new IOutputSink() {
      @Override
      public void close() throws IOException {
      }

      @Override
      public void begin() {
      }

      @Override
      public void acceptOther(String path) {
        saver.copyFile(file.getAbsolutePath(), "", path);
      }

      @Override
      public void acceptDirectory(String directory) {
        // The input class is intentionally flattened for compatibility. Create
        // directories only when acceptClass establishes that an output is not it.
      }

      @Override
      public void acceptClass(String qualifiedName, String fileName, String content, int[] mapping) {
        boolean inputClass = qualifiedName.equals(SingleFileContextSource.this.qualifiedName);
        String entryName = inputClass ? fileName.substring(fileName.lastIndexOf('/') + 1) : fileName;
        if (!inputClass) {
          int slash = entryName.lastIndexOf('/');
          if (slash > 0) saver.saveFolder(entryName.substring(0, slash));
        }
        saver.saveClassFile("", qualifiedName, entryName, content, mapping);
      }
    };
  }


}
