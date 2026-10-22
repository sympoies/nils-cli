# A real mmap-only holder: Python mmap.mmap normally duplicates the descriptor.
import ctypes
import mmap
import sys
import time

libc = ctypes.CDLL(None, use_errno=True)
libc.mmap.restype = ctypes.c_void_p
libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int,
                     ctypes.c_int, ctypes.c_int, ctypes.c_longlong]
with open(sys.argv[1], "rb") as source:
    address = libc.mmap(None, 1, mmap.PROT_READ, mmap.MAP_PRIVATE,
                        source.fileno(), 0)
    assert address not in (None, ctypes.c_void_p(-1).value), ctypes.get_errno()
    ctypes.string_at(address, 1)
# The mapping exists and its page has been read; the sole descriptor is closed.
print("ready", flush=True)
time.sleep(30)
