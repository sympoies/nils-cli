#!/usr/bin/env python3
import os
import pathlib
import signal
import sys
import time

args = sys.argv[1:]
with open(os.environ["SOPS_LOG"], "a") as log:
    log.write(" ".join(args) + "\n")
mode = os.environ.get("SOPS_MODE", "")
encrypt = "-e" in args
source = args[-1]
raw = sys.stdin.buffer.read() if source == "/dev/stdin" else pathlib.Path(source).read_bytes()
if (encrypt and mode == "encrypt-fail") or (not encrypt and mode == "decrypt-fail"):
    sys.stderr.write(os.environ["SYNTHETIC_CANARY"])
    sys.exit(1)
if encrypt:
    if mode == "interrupt":
        pathlib.Path(os.environ["READY_FILE"]).write_text("ready")
        time.sleep(30)
    if mode == "plaintext":
        sys.stdout.buffer.write(raw)
        sys.exit(0)
    for line in raw.decode().splitlines():
        if not line or line.startswith("#"):
            print(line)
        else:
            key, value = line.split("=", 1)
            print(key + "=ENC[AES256_GCM,data:" + value.encode().hex() + "]")
    final = args[args.index("--filename-override") + 1]
    recipient = "synthetic-final-recipient" if final == "repos/service.enc.env" else "synthetic-wrong-recipient"
    recipient = os.environ.get("SOPS_AGE_RECIPIENTS", recipient)
    print("sops_age__list_0__map_recipient=" + recipient)
    print("sops_mac=ENC[AES256_GCM,data:synthetic]")
else:
    if mode == "interrupt-validation" and ".tmp" in source:
        pathlib.Path(os.environ["READY_FILE"]).write_text("ready")
        time.sleep(30)
    if mode == "validate-fail" and ".tmp" in source:
        sys.stderr.write(os.environ["SYNTHETIC_CANARY"])
        sys.exit(1)
    try:
        for line in raw.decode().splitlines():
            if line.startswith("sops_"):
                continue
            if not line or line.startswith("#"):
                print(line)
            else:
                key, value = line.split("=ENC[AES256_GCM,data:", 1)
                print(key + "=" + bytes.fromhex(value[:-1]).decode())
    except Exception:
        sys.stderr.write(os.environ["SYNTHETIC_CANARY"])
        sys.exit(1)
