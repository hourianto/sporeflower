"""Reviewed monitor cleanup with two exception entries sharing a stack phi.

The second handler retries monitorexit. A proven owned release cannot take that
edge; cleanup must nevertheless preserve the two distinct exception ValueIds.
The clear variant deliberately replaces the exception with null before release.
"""
from pathlib import Path
from struct import pack

pool, memo = [], {}
def entry(key, value):
    if key not in memo:
        pool.append(value)
        memo[key] = len(pool)
    return memo[key]
def utf8(value):
    data = value.encode()
    return entry(('u', value), b'\x01' + pack('>H', len(data)) + data)
def cls(value):
    return entry(('c', value), b'\x07' + pack('>H', utf8(value)))
def nt(name, descriptor):
    return entry(('n', name, descriptor), b'\x0c' + pack('>HH', utf8(name), utf8(descriptor)))
def method(owner, name, descriptor):
    return entry(('m', owner, name, descriptor), b'\x0a' + pack('>HH', cls(owner), nt(name, descriptor)))

classes = [cls('MonitorCopies'), cls('MonitorCopiesCleared')]
super_class = cls('java/lang/Object')
code_name = utf8('Code')
step = method('EHSupport', 'step', '(II)I')
methods = []
for name in ['run', 'clear']:
    code, labels, fixups = bytearray(), {}, []
    def mark(label): labels[label] = len(code)
    def emit(*values): code.extend(values)
    def jump(label):
        fixups.append((len(code), label))
        emit(0xa7, 0, 0)
    def call(number):
        emit(3 + number, 0x1b, 0xb8)
        code.extend(pack('>H', step))
        emit(0x57)
    emit(0x2a, 0xc2)  # aload_0; monitorenter
    mark('body')
    call(1)
    emit(0x2a, 0xc3)  # aload_0; monitorexit
    mark('body_end')
    call(2)
    emit(0xb1)
    mark('caught')
    if name == 'clear': emit(0x57, 0x01)  # discard exception, replace with null
    jump('release')
    mark('retry')
    jump('release')
    mark('release')
    emit(0x2a, 0xc3)
    mark('release_end')
    emit(0xbf)  # athrow: still has the exception from the actual incoming edge
    for pc, label in fixups:
        code[pc + 1:pc + 3] = pack('>h', labels[label] - pc)
    handlers = [
        (labels['body'], labels['body_end'], labels['caught'], 0),
        (labels['release'], labels['release_end'], labels['retry'], 0),
    ]
    attribute = pack('>HHI', 2, 2, len(code)) + code + pack('>H', len(handlers))
    attribute += b''.join(pack('>HHHH', *handler) for handler in handlers) + b'\x00\x00'
    methods.append(pack('>HHHHHI', 0x0009, utf8(name), utf8('(Ljava/lang/Object;I)V'), 1, code_name, len(attribute)) + attribute)

destination = Path(__file__).with_name('classes')
destination.mkdir(exist_ok=True)
for name, this_class, method_info in zip(['MonitorCopies', 'MonitorCopiesCleared'], classes, methods):
    data = pack('>IHHH', 0xcafebabe, 0, 49, len(pool) + 1) + b''.join(pool)
    data += pack('>HHHHHH', 0x0031, this_class, super_class, 0, 0, 1)
    data += method_info + b'\x00\x00'
    destination.joinpath(name + '.class').write_bytes(data)
