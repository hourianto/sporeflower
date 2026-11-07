"""Generate one reviewed, side-effect-bounded version-49 classfile.

Only calls EHSupport.step and EHSupport.identity. The first handler catches
step(1), and its step(2) handler shares a target with the normal mode<0 path.
"""
from pathlib import Path
from struct import pack

pool = []
memo = {}
def entry(key, data):
    if key not in memo:
        pool.append(data)
        memo[key] = len(pool)
    return memo[key]
def utf8(value):
    raw = value.encode()
    return entry(('u', value), b'\x01' + pack('>H', len(raw)) + raw)
def cls(value):
    return entry(('c', value), b'\x07' + pack('>H', utf8(value)))
def nt(name, descriptor):
    return entry(('n', name, descriptor), b'\x0c' + pack('>HH', utf8(name), utf8(descriptor)))
def method(owner, name, descriptor):
    return entry(('m', owner, name, descriptor), b'\x0a' + pack('>HH', cls(owner), nt(name, descriptor)))
def field(owner, name, descriptor):
    return entry(('f', owner, name, descriptor), b'\x09' + pack('>HH', cls(owner), nt(name, descriptor)))

this_class = cls('SharedExceptionalExit')
super_class = cls('java/lang/Object')
code_name = utf8('Code')
method_name, method_type = utf8('run'), utf8('(II)I')
step = method('EHSupport', 'step', '(II)I')
identity = method('EHSupport', 'identity', '(Ljava/lang/Throwable;)I')
failures = field('EHSupport', 'failures', '[Ljava/lang/RuntimeException;')
iae, rte = cls('java/lang/IllegalArgumentException'), cls('java/lang/RuntimeException')

code = bytearray()
labels = {}
fixups = []
def mark(name): labels[name] = len(code)
def emit(*values): code.extend(values)
def ref(opcode, index): code.extend(bytes([opcode]) + pack('>H', index))
def branch(opcode, target):
    fixups.append((len(code), target))
    emit(opcode, 0, 0)
def call_step(number):
    emit(3 + number, 0x1b)  # iconst_n; iload_1
    ref(0xb8, step)
    emit(0x57)  # pop

emit(0x03, 0x3d)  # result = 0
mark('try_start')
call_step(1)
emit(0x1a)  # iload_0
branch(0x9b, 'normal_shared')  # iflt
emit(0x05, 0x3d)  # result = 2
mark('try_end')
branch(0xa7, 'follow')
mark('normal_shared')
ref(0xb2, failures)
emit(0x03, 0x32)  # failures[0]
branch(0xa7, 'shared')
mark('initial_catch')
emit(0x4e)  # astore_3
mark('catch_start')
call_step(2)
emit(0x10, 30, 0x3d)  # result = 30
mark('catch_end')
branch(0xa7, 'follow')
mark('shared')
emit(0x4e)  # astore_3 (normal and exceptional incoming values)
call_step(3)
emit(0x2d)  # aload_3
ref(0xb8, identity)
emit(0x10, 40, 0x60, 0x3d)  # result = identity(exception) + 40
branch(0xa7, 'follow')
mark('follow')
call_step(4)
emit(0x1c, 0xac)  # return result

for pc, target in fixups:
    code[pc + 1:pc + 3] = pack('>h', labels[target] - pc)
handlers = [
    (labels['try_start'], labels['try_end'], labels['initial_catch'], iae),
    (labels['catch_start'], labels['catch_end'], labels['shared'], rte),
]
attribute = pack('>HHI', 2, 4, len(code)) + code + pack('>H', len(handlers))
attribute += b''.join(pack('>HHHH', *handler) for handler in handlers) + b'\x00\x00'
method_info = pack('>HHHHHI', 0x0009, method_name, method_type, 1, code_name, len(attribute)) + attribute
classfile = pack('>IHHH', 0xcafebabe, 0, 49, len(pool) + 1) + b''.join(pool)
classfile += pack('>HHHHHH', 0x0031, this_class, super_class, 0, 0, 1) + method_info + b'\x00\x00'
Path(__file__).with_name('classes').joinpath('SharedExceptionalExit.class').write_bytes(classfile)
print('Labels:', labels)
print('Exception table:', handlers)
