#!/usr/bin/env python3
"""End-to-end smoke test of Grim-Horizon on a real Horizon dedicated server.

It starts the server built by tools/build_server.py in a fresh scratch folder
(never your own worlds), checks that the anti-cheat loaded and answers console
commands, connects Horizon's own protocol bots (horizon-bots), and checks the
server log:

  * the bots answer the anti-cheat's ping transactions (no timeout warnings),
  * the bots, which fly upwards in a survival world without being allowed to,
    are flagged by the movement checks (Simulation),
  * the server stays up and stops cleanly.

It needs the prepared Minecraft 26.3 resources for Horizon (made from your own
Minecraft files by `horizon-prepare --prepare`, see BUILD.md) and the
`horizon-bots` binary from the same staged workspace
(`cargo build --profile fast -p horizon-bots` in work/staged).

The server refuses to run until its eula.txt says eula=true. Accepting the
Minecraft EULA (https://aka.ms/MinecraftEULA) is your decision: pass
--accept-eula to have this script write eula=true in the scratch server folder.

Usage:
  python3 tools/smoke_test.py --resources /path/to/.local/horizon-client/26.3 \
      --bots work/target/fast/horizon-bots --accept-eula [--server dist/horizon-engine-grim]
"""
import argparse
import os
import queue
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", type=Path, default=ROOT / "dist" / "horizon-engine-grim")
    ap.add_argument("--resources", type=Path, required=True)
    ap.add_argument("--bots", type=Path, required=True)
    ap.add_argument("--accept-eula", action="store_true", help="write eula=true in the scratch server folder")
    ap.add_argument("--seconds", type=int, default=25)
    ap.add_argument("--keep", action="store_true", help="keep the scratch folder")
    args = ap.parse_args()
    if not args.accept_eula:
        sys.exit("The server needs eula=true; pass --accept-eula if you accept https://aka.ms/MinecraftEULA")

    scratch = Path(tempfile.mkdtemp(prefix="grim-horizon-smoke-"))
    home = scratch / "home"
    for d in ["home", "xdg-data", "xdg-config", "server"]:
        (scratch / d).mkdir()
    server_dir = scratch / "server"
    port = free_port()
    (server_dir / "eula.txt").write_text("eula=true\n")
    (server_dir / "server.properties").write_text(
        "\n".join(
            [
                "online-mode=false",
                "white-list=false",
                f"server-port={port}",
                "gamemode=survival",
                "difficulty=peaceful",
                "level-seed=12345",
                "view-distance=4",
                "simulation-distance=4",
                "spawn-protection=0",
            ]
        )
        + "\n"
    )
    env = dict(
        os.environ,
        HORIZON_HOME=str(home),
        XDG_DATA_HOME=str(scratch / "xdg-data"),
        XDG_CONFIG_HOME=str(scratch / "xdg-config"),
        HORIZON_NO_AUDIO="1",
        HORIZON_LAUNCHER_NO_SELF_UPDATE="1",
    )
    cmd = [str(args.server.resolve()), "--resources", str(args.resources.resolve()), "--dedicated", "--nogui", "--port", str(port), "--world", "smoke"]
    print("+", " ".join(cmd), f"(in {server_dir})")
    proc = subprocess.Popen(cmd, cwd=server_dir, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    lines: "queue.Queue[str]" = queue.Queue()
    log = []

    def pump():
        for line in proc.stdout:
            log.append(line.rstrip("\n"))
            lines.put(line.rstrip("\n"))

    threading.Thread(target=pump, daemon=True).start()

    def wait_for(pattern, timeout):
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = lines.get(timeout=0.5)
            except queue.Empty:
                if proc.poll() is not None:
                    return None
                continue
            if re.search(pattern, line):
                return line
        return None

    def console(line):
        proc.stdin.write(line + "\n")
        proc.stdin.flush()

    failures = []
    try:
        if not wait_for(r"Done \(", 300):
            failures.append("the server did not start (no Done line)")
            raise SystemExit
        print("server is up on port", port)
        console("grim checks")
        if not wait_for(r"Simulation \[on\]", 20):
            failures.append("`grim checks` was not answered on the console")
        bots = subprocess.run(
            [str(args.bots), "--server", f"127.0.0.1:{port}", "--bots", "2", "--behaviour", "walk", "--seconds", str(args.seconds), "--name", "GrimBot"],
            capture_output=True,
            text=True,
            timeout=args.seconds + 120,
        )
        print(bots.stdout[-2000:])
        console("grim profile GrimBot0")
        time.sleep(2)
        console("stop")
        proc.wait(timeout=120)
    except SystemExit:
        pass
    finally:
        if proc.poll() is None:
            proc.kill()

    text = "\n".join(log)
    (ROOT / "work").mkdir(exist_ok=True)
    (ROOT / "work" / "smoke-server.log").write_text(text)
    loaded = "Grim-Horizon" in text
    alerts = re.findall(r"\[Grim\] GrimBot\d failed (\w+)", text)
    timeouts = "has not answered transactions" in text
    print(f"anti-cheat loaded: {loaded}")
    print(f"alerts: {len(alerts)} ({', '.join(sorted(set(alerts)))})")
    if not loaded:
        failures.append("no Grim-Horizon start line in the log")
    if not any(a == "Simulation" for a in alerts):
        failures.append("the flying bots were not flagged by Simulation")
    if timeouts:
        failures.append("bots were reported as not answering transactions")
    if proc.returncode not in (0, None):
        failures.append(f"server exited with {proc.returncode}")
    if not args.keep:
        import shutil

        shutil.rmtree(scratch, ignore_errors=True)
    print("server log saved to work/smoke-server.log")
    if failures:
        print("SMOKE TEST FAILED:\n  " + "\n  ".join(failures))
        sys.exit(1)
    print("SMOKE TEST PASSED")


if __name__ == "__main__":
    main()
