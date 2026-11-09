"""Regenerate DelayedLiteralStore.class; tests consume the committed class.

The verifier accepts a literal held on the operand stack across a throwing call.
The old slot binding must remain visible to the handler until istore commits.
A later branch joins 7 and 9, and its result is used twice (x * x), exercising
same-block literal-default capture even when single-use choices stay inline.
"""
from pathlib import Path
from struct import pack
root = Path(__file__).resolve().parent
out = root / 'classes'
out.mkdir(exist_ok=True)
pool, memo = [], {}
def entry(key, value):
    if key not in memo:
        pool.append(value); memo[key] = len(pool)
    return memo[key]
def utf8(value):
    data = value.encode()
    return entry(('u', value), b'\x01' + pack('>H', len(data)) + data)
def cls(value): return entry(('c', value), b'\x07' + pack('>H', utf8(value)))
def nt(name, descriptor): return entry(('n', name, descriptor), b'\x0c' + pack('>HH', utf8(name), utf8(descriptor)))
def method(owner, name, descriptor): return entry(('m', owner, name, descriptor), b'\x0a' + pack('>HH', cls(owner), nt(name, descriptor)))
this_class, super_class = cls('DelayedLiteralStore'), cls('java/lang/Object')
code_name, step = utf8('Code'), method('DelayedLiteralStoreDriver', 'step', '(II)V')
exception = cls('java/lang/RuntimeException')
code, labels, fixups = bytearray(), {}, []
def emit(*values): code.extend(values)
def mark(label): labels[label] = len(code)
def jump(op, label): fixups.append((len(code), label)); emit(op, 0, 0)
def call(bit):
    emit(0x1a, 0x10, bit, 0xb8); code.extend(pack('>H', step))
emit(0x1a, 0x10, 33, 0x60, 0x3c)  # old slot value = flags + 33
mark('try')
emit(0x10, 7)  # future x value waits below call arguments
call(1)
emit(0x3c)  # commit only after first call succeeds
emit(0x1a, 0x07, 0x7e)  # iload_0, iconst_4, iand
jump(0x99, 'join')
emit(0x10, 9, 0x3c)
mark('join')
emit(0x1b, 0x1b, 0x68, 0xac)  # joined x has two uses: x * x
mark('end')
mark('catch')
emit(0x4d, 0x1b, 0xac)  # astore_2; iload_1; ireturn
for pc, label in fixups: code[pc+1:pc+3] = pack('>h', labels[label]-pc)
handlers = [(labels['try'], labels['end'], labels['catch'], exception)]
attribute = pack('>HHI', 3, 3, len(code)) + code + pack('>H', len(handlers))
attribute += b''.join(pack('>HHHH', *h) for h in handlers) + b'\x00\x00'
method_info = pack('>HHHHHI', 0x0009, utf8('run'), utf8('(I)I'), 1, code_name, len(attribute)) + attribute
data = pack('>IHHH', 0xcafebabe, 0, 49, len(pool)+1) + b''.join(pool)
data += pack('>HHHHHH', 0x0031, this_class, super_class, 0, 0, 1) + method_info + b'\x00\x00'
(out / 'DelayedLiteralStore.class').write_bytes(data)
