#!/usr/bin/env python3
"""Read-only Darwin process-marker diagnostics; never prints argv/env values."""
import ctypes
import errno
import json
import os
import pathlib
import platform
import struct
import subprocess
import sys
import time
import uuid

if platform.system() != "Darwin":
    raise SystemExit("This diagnostic requires macOS")
libc = ctypes.CDLL(None, use_errno=True)
libc.sysctl.argtypes = [ctypes.POINTER(ctypes.c_int), ctypes.c_uint,
                       ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
                       ctypes.c_void_p, ctypes.c_size_t]
marker_id, marker_runtime = "marker-probe", str(uuid.uuid4())
environment = dict(os.environ, AGENT_SESSION_ID=marker_id,
                   AGENT_SESSION_RUNTIME_ID=marker_runtime)
commands = [("platform-sleep", ["/bin/sleep", "30"]),
            ("platform-python", ["/usr/bin/python3", "-c", "import time; time.sleep(30)"]),
            ("runner-python", [sys.executable, "-c", "import time; time.sleep(30)"])]
for child_kind, command in commands:
    child = subprocess.Popen(command, env=environment, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
    try:
        time.sleep(0.1)
        for capacity in [1024 * 1024, 256 * 1024]:
            mib = (ctypes.c_int * 3)(1, 49, child.pid)
            buffer = ctypes.create_string_buffer(capacity)
            length = ctypes.c_size_t(capacity)
            result = libc.sysctl(mib, 3, buffer, ctypes.byref(length), None, 0)
            error = ctypes.get_errno() if result != 0 else 0
            report = dict(child_kind=child_kind, executable=pathlib.Path(command[0]).name,
                          pid=child.pid, capacity=capacity, result=result,
                          errno=error, errno_name=errno.errorcode.get(error),
                          length=length.value, child_live=child.poll() is None)
            if result == 0:
                data = buffer.raw[:length.value]
                argc = struct.unpack_from("i", data)[0]
                offset = data.index(b"\0", 4) + 1
                while offset < len(data) and data[offset] == 0:
                    offset += 1
                for _ in range(argc):
                    offset = data.index(b"\0", offset) + 1
                entries = data[offset:].split(b"\0")
                report.update(argc=argc, environment_entries=len(entries),
                              id_present=("AGENT_SESSION_ID=" + marker_id).encode() in entries,
                              runtime_present=("AGENT_SESSION_RUNTIME_ID=" + marker_runtime).encode() in entries)
            print(json.dumps(report), flush=True)
    finally:
        child.terminate()
        child.wait(timeout=5)
