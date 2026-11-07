#!/usr/bin/env python3
"""Compile the neutral oracle, then encode two valid legacy allocation graphs.

No extra dependencies: the patcher changes only the named Code attributes.
Existing source and all other declarations remain Java compiler output.
"""
from pathlib import Path
import argparse
import struct
import subprocess


def u2(number):
    return struct.pack(">H", number)


def u4(number):
    return struct.pack(">I", number)


class Code:
    def __init__(self):
        self.data = bytearray()
        self.labels = {}
        self.branches = []

    def op(self, opcode, operand=b""):
        self.data.append(opcode)
        self.data.extend(operand)

    def label(self, name):
        self.labels[name] = len(self.data)

    def branch(self, opcode, name):
        self.branches.append((len(self.data), name))
        self.op(opcode, b"\0\0")

    def finish(self):
        for pc, name in self.branches:
            self.data[pc + 1:pc + 3] = struct.pack(">h", self.labels[name] - pc)
        return bytes(self.data)


def patch(path):
    data = path.read_bytes()
    count = struct.unpack_from(">H", data, 8)[0]
    cp = [None] * count
    position = 10
    index = 1
    while index < count:
        tag = data[position]
        position += 1
        if tag == 1:
            length = struct.unpack_from(">H", data, position)[0]
            position += 2
            cp[index] = (tag, data[position:position + length].decode("utf-8"))
            position += length
        elif tag in (7, 8, 16):
            cp[index] = (tag, struct.unpack_from(">H", data, position)[0])
            position += 2
        elif tag in (9, 10, 11, 12, 18):
            cp[index] = (tag, *struct.unpack_from(">HH", data, position))
            position += 4
        elif tag in (3, 4):
            position += 4
        elif tag in (5, 6):
            position += 8
            index += 1
        elif tag == 15:
            position += 3
        else:
            raise ValueError(f"unexpected constant tag {tag}")
        index += 1

    def text(index):
        return cp[index][1]

    def class_index(name):
        return next(i for i, item in enumerate(cp) if item and item[0] == 7 and text(item[1]) == name)

    def member(tag, owner, name, descriptor):
        for i, item in enumerate(cp):
            if not item or item[0] != tag:
                continue
            nat = cp[item[2]]
            if text(cp[item[1]][1]) == owner and text(nat[1]) == name and text(nat[2]) == descriptor:
                return i
        raise ValueError((tag, owner, name, descriptor))

    owner = "AllocationGraphProbe"
    loop_owner = owner + "$Loop"
    code = Code()
    code.op(0xbb, u2(class_index(loop_owner)))  # new Loop
    code.op(0x4b)                            # astore_0
    code.op(0x03)                            # iconst_0
    code.op(0x3c)                            # istore_1
    code.label("test")
    code.op(0x1b)
    code.op(0xb2, u2(member(9, owner, "limit", "I")))
    code.branch(0xa2, "init")
    code.op(0xb8, u2(member(10, owner, "effectLoop", "()V")))
    code.op(0x84, bytes([1, 1]))
    code.branch(0xa7, "test")
    code.label("init")
    code.op(0x2a)
    code.op(0x1b)
    code.op(0xb7, u2(member(10, loop_owner, "<init>", "(I)V")))
    code.op(0x2a)
    code.op(0xb0)
    loop = code.finish()

    choice_owner = owner + "$Choice"
    code = Code()
    code.op(0xbb, u2(class_index(choice_owner)))
    code.op(0x4b)
    code.op(0xb2, u2(member(9, owner, "choiceFlag", "Z")))
    code.branch(0x99, "string")
    code.op(0x2a)
    code.op(0x04)
    code.op(0xb7, u2(member(10, choice_owner, "<init>", "(I)V")))
    code.branch(0xa7, "done")
    code.label("string")
    code.op(0x2a)
    string = next(i for i, item in enumerate(cp) if item and item[0] == 8 and text(item[1]) == "two")
    code.op(0x13, u2(string))
    code.op(0xb7, u2(member(10, choice_owner, "<init>", "(Ljava/lang/String;)V")))
    code.label("done")
    code.op(0x2a)
    code.op(0xb0)
    alternatives = code.finish()

    # Skip interfaces, then fields and their attributes.
    position += 6
    interfaces = struct.unpack_from(">H", data, position)[0]
    position += 2 + 2 * interfaces

    def attributes(position):
        count = struct.unpack_from(">H", data, position)[0]
        position += 2
        result = []
        for _ in range(count):
            name, length = struct.unpack_from(">HI", data, position)
            position += 6
            result.append((name, data[position:position + length]))
            position += length
        return position, result

    fields = struct.unpack_from(">H", data, position)[0]
    position += 2
    for _ in range(fields):
        position += 6
        position, _ = attributes(position)
    methods = struct.unpack_from(">H", data, position)[0]
    position += 2
    output = bytearray(data[:position])
    # The patched graph deliberately has no StackMapTable; legacy inference
    # verifies uninitialized aliases directly.
    output[6:8] = u2(49)
    for _ in range(methods):
        header = data[position:position + 6]
        name = text(struct.unpack_from(">H", header, 2)[0])
        position, attrs = attributes(position + 6)
        replacement = {"loop": (loop, 2, 2), "alternatives": (alternatives, 2, 1)}.get(name)
        if replacement:
            bytecode, stack, locals_ = replacement
            payload = u2(stack) + u2(locals_) + u4(len(bytecode)) + bytecode + u2(0) + u2(0)
            attrs = [(index, payload if text(index) == "Code" else content) for index, content in attrs]
        output.extend(header)
        output.extend(u2(len(attrs)))
        for index, content in attrs:
            output.extend(u2(index) + u4(len(content)) + content)
    output.extend(data[position:])
    path.write_bytes(output)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--javac", default="javac")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    output = root / "classes"
    output.mkdir(exist_ok=True)
    subprocess.run([args.javac, "-source", "7", "-target", "7", "-Xlint:-options", "-g:none", "-d", str(output), str(root / "AllocationGraphProbe.java")], check=True)
    patch(output / "AllocationGraphProbe.class")


if __name__ == "__main__":
    main()
