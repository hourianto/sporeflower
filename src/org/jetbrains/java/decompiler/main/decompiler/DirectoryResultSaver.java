package org.jetbrains.java.decompiler.main.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IResultSaver;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.io.*;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.jar.Manifest;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;

public class DirectoryResultSaver implements IResultSaver {
  private final Path root;

  public DirectoryResultSaver(File root) {
    this.root = root.toPath().toAbsolutePath().normalize();
  }

  @Override
  public void saveClassEntry(String path, String archiveName, String qualifiedName, String entryName, String content) {
    Path entryPath = SavePathUtil.resolve(this.root, "class entry", entryName);
    if (entryPath == null) {
      return;
    }

    try {
      Path parent = entryPath.getParent();
      if (parent != null) {
        Files.createDirectories(parent);
      }
    } catch (IOException e) {
      throw new RuntimeException("Failed to create class parent directory", e);
    }

    try (BufferedWriter writer = Files.newBufferedWriter(entryPath)) {
      if (content != null) {
        writer.write(content);
      }
    } catch (IOException e) {
      throw new RuntimeException("Failed to save class", e);
    }
  }

  @Override
  public void saveDirEntry(String path, String archiveName, String entryName) {
    Path entryPath = SavePathUtil.resolve(this.root, "directory entry", entryName);
    if (entryPath == null) {
      return;
    }

    try {
      Files.createDirectories(entryPath);
    } catch (IOException e) {
      throw new RuntimeException("Failed to save directory", e);
    }
  }

  @Override
  public void createArchive(String path, String archiveName, Manifest manifest) {

  }

  @Override
  public void saveFolder(String path) {
    Path entryPath = SavePathUtil.resolve(this.root, "folder", path);
    if (entryPath == null) {
      return;
    }

    try {
      Files.createDirectories(entryPath);
    } catch (IOException e) {
      throw new RuntimeException("Failed to save directory", e);
    }
  }

  @Override
  public void copyFile(String source, String path, String entryName) {
    Path target = SavePathUtil.resolve(this.root, "file", path, entryName);
    if (target == null) {
      return;
    }

    try {
      InterpreterUtil.copyFile(new File(source), target.toFile());
    }
    catch (IOException ex) {
      DecompilerContext.getLogger().writeMessage("Cannot copy " + source + " to " + entryName, ex);
    }
  }

  @Override
  public void saveClassFile(String path, String qualifiedName, String entryName, String content, int[] mapping) {
    Path entryPath = SavePathUtil.resolve(this.root, "class file", path, entryName);
    if (entryPath == null) {
      return;
    }

    try {
      Path parent = entryPath.getParent();
      if (parent != null) {
        Files.createDirectories(parent);
      }
    } catch (IOException e) {
      throw new RuntimeException("Failed to create class parent directory", e);
    }

    try (BufferedWriter writer = Files.newBufferedWriter(entryPath)) {
      if (content != null) {
        writer.write(content);
      }
    } catch (IOException e) {
      throw new RuntimeException("Failed to save class", e);
    }
  }

  @Override
  public void copyEntry(String source, String path, String archiveName, String entryName) {
    Path target = SavePathUtil.resolve(this.root, "archive entry", path, entryName);
    if (target == null) {
      return;
    }

    try (ZipFile srcArchive = new ZipFile(new File(source))) {
      ZipEntry entry = srcArchive.getEntry(entryName);
      if (entry != null) {
        try (InputStream in = srcArchive.getInputStream(entry); OutputStream out = Files.newOutputStream(target)) {
          InterpreterUtil.copyStream(in, out);
        }
      }
    }
    catch (IOException ex) {
      String message = "Cannot copy entry " + entryName + " from " + source;
      DecompilerContext.getLogger().writeMessage(message, ex);
    }
  }

  @Override
  public void closeArchive(String path, String archiveName) {

  }
}
