#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""End-to-end test of Grim-Horizon on a real Horizon dedicated server.

It starts the server built by tools/build_server.py in a fresh scratch folder
(never your own worlds: HORIZON_HOME, XDG folders and the server folder are
all new temporary folders), on a superflat world, and drives it with grim-bot
players (tools/grim-bot): the same vanilla-model client the engine's tests
use, speaking the real protocol. Every assertion is reported on its own line.

  Phase A, alert-only (the default mode)
    startup, the anti-cheat answering console commands, players joining,
    a legitimate player walking, jumping, sprinting and sneaking with no
    alert, a speed cheater flagged by Simulation, staff alerts reaching an
    operator in game, a console teleport, death and respawn, a trip to the
    Nether and back, ping transactions answered.
  Phase B, setback mode (`grim reload` after editing grim-horizon.toml)
    a cheater pulled back by setbacks; the legitimate player never.
  Phase C, punish mode
    a cheater kicked, the same name reconnecting, judged and kicked again;
    the legitimate player reconnecting and still clean.
  Shutdown
    `stop` ends the server cleanly.

What it needs, all prepared by you (see BUILD.md, "Live test"):
  * --resources: Minecraft 26.3 game files for Horizon, made by
    `horizon-prepare --prepare --server-jar ...` from your own Minecraft copy;
  * the server: `python3 tools/build_server.py ...` (dist/horizon-engine-grim);
  * --accept-eula: the server refuses to run until eula.txt says eula=true.
    Accepting the Minecraft EULA (https://aka.ms/MinecraftEULA) is your
    decision; this flag writes eula=true in the scratch server folder only.

Without --accept-eula, or without resources, nothing is started and the
result is NOT RUN, never PASSED.

Usage:
  python3 tools/smoke_test.py --resources /path/to/horizon-client/26.3 --accept-eula
"""
import argparse
import os
import queue
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BOT_DIR = ROOT / "tools" / "grim-bot"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def grim_config(mode, kick_vl=None):
    text = (ROOT / "configs" / "grim-horizon.toml").read_text()
    text = re.sub(r'^mode = ".*"$', f'mode = "{mode}"', text, count=1, flags=re.M)
    if kick_vl is not None:
        text += f"\n[checks.Simulation]\nkick_vl = {kick_vl}\n"
    return text


class Server:
    def __init__(self, cmd, cwd, env):
        print("+", " ".join(cmd), f"(in {cwd})")
        self.proc = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        self.log = []
        self.lines = queue.Queue()
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        for line in self.proc.stdout:
            line = line.rstrip("\n")
            self.log.append(line)
            self.lines.put(line)

    def wait_for(self, pattern, timeout):
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = self.lines.get(timeout=0.5)
            except queue.Empty:
                if self.proc.poll() is not None:
                    return None
                continue
            if re.search(pattern, line):
                return line
        return None

    def console(self, line):
        print("> " + line)
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def text(self):
        return "\n".join(self.log)


class Bot:
    def __init__(self, exe, port, name, mode, seconds):
        self.name = name
        self.out = []
        self.proc = subprocess.Popen(
            [str(exe), "--server", f"127.0.0.1:{port}", "--name", name, "--mode", mode, "--seconds", str(seconds)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        for line in self.proc.stdout:
            self.out.append(line.rstrip("\n"))

    def lines(self, prefix):
        return [l for l in list(self.out) if l.startswith(prefix)]

    def wait_for(self, prefix, timeout, count=1):
        end = time.time() + timeout
        while time.time() < end:
            if len(self.lines(prefix)) >= count:
                return True
            if self.proc.poll() is not None and len(self.lines(prefix)) < count:
                time.sleep(0.2)
                return len(self.lines(prefix)) >= count
            time.sleep(0.2)
        return False

    def stop(self):
        try:
            self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill()


class Results:
    def __init__(self):
        self.items = []

    def check(self, name, ok, detail=""):
        self.items.append((name, bool(ok), detail))
        print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f": {detail}" if detail and not ok else ""))
        return ok


def alerts(server, player):
    return re.findall(rf"\[Grim\] {player} failed (\w+)", server.text())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", type=Path, default=ROOT / "dist" / "horizon-engine-grim")
    ap.add_argument("--resources", type=Path, required=True)
    ap.add_argument("--bot", type=Path, default=BOT_DIR / "target" / "release" / "grim-bot")
    ap.add_argument("--accept-eula", action="store_true", help="write eula=true in the scratch server folder")
    ap.add_argument("--keep", action="store_true", help="keep the scratch folder")
    args = ap.parse_args()
    if not args.accept_eula:
        sys.exit("NOT RUN: the server needs eula=true; pass --accept-eula if you accept https://aka.ms/MinecraftEULA")
    if not (args.resources / "items.json").exists():
        sys.exit(f"NOT RUN: {args.resources} holds no prepared 26.3 game files (items.json); see BUILD.md")
    if not args.server.exists():
        sys.exit(f"NOT RUN: {args.server} not found; run tools/build_server.py first")
    if not args.bot.exists():
        print("building grim-bot")
        subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(BOT_DIR / "Cargo.toml")], check=True)

    scratch = Path(tempfile.mkdtemp(prefix="grim-horizon-smoke-"))
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
                "level-type=minecraft\\:flat",
                "generate-structures=false",
                "level-seed=12345",
                "view-distance=4",
                "simulation-distance=4",
                "spawn-protection=0",
            ]
        )
        + "\n"
    )
    config = server_dir / "grim-horizon.toml"
    config.write_text(grim_config("alert-only"))
    env = dict(
        os.environ,
        HORIZON_HOME=str(scratch / "home"),
        XDG_DATA_HOME=str(scratch / "xdg-data"),
        XDG_CONFIG_HOME=str(scratch / "xdg-config"),
        HORIZON_NO_AUDIO="1",
        HORIZON_LAUNCHER_NO_SELF_UPDATE="1",
    )
    cmd = [str(args.server.resolve()), "--resources", str(args.resources.resolve()), "--dedicated", "--nogui", "--port", str(port), "--world", "smoke"]
    server = Server(cmd, server_dir, env)
    r = Results()
    bots = []

    def bot(name, mode, seconds):
        b = Bot(args.bot, port, name, mode, seconds)
        bots.append(b)
        return b

    try:
        if not r.check("server starts (Done line)", server.wait_for(r"Done \(", 300)):
            raise SystemExit
        server.console("grim checks")
        r.check("console: grim checks", server.wait_for(r"Simulation \[on\]", 20))
        # The engine starts on first use and says so (printed on the next tick).
        r.check("anti-cheat loaded", server.wait_for(r"Grim-Horizon .* enabled", 10) or "Grim-Horizon" in server.text())

        # --- Phase A: alert-only.
        staff = bot("GrimStaff", "stand", 150)
        legit = bot("GrimLegit", "legit", 75)
        r.check("players join and spawn", staff.wait_for("SPAWNED", 60) and legit.wait_for("SPAWNED", 60))
        server.console("op GrimStaff")
        time.sleep(2)
        speed = bot("GrimSpeed", "speed", 20)
        r.check("cheater joins", speed.wait_for("SPAWNED", 60))
        time.sleep(8)
        server.console("tp GrimLegit ~3 ~ ~3")
        r.check("console teleport acknowledged", legit.wait_for("TELEPORT", 20, count=2))
        time.sleep(5)
        server.console("kill GrimLegit")
        r.check("death and respawn", legit.wait_for("RESPAWN minecraft:overworld", 20) and legit.wait_for("SPAWNED", 20, count=2))
        time.sleep(5)
        server.console("execute in minecraft:the_nether run tp GrimLegit 0.5 130 0.5")
        r.check("dimension change to the Nether", legit.wait_for("RESPAWN minecraft:the_nether", 30))
        time.sleep(8)
        server.console("execute in minecraft:overworld run tp GrimLegit 0.5 -60 0.5")
        r.check("dimension change back", legit.wait_for("RESPAWN minecraft:overworld", 30, count=2))
        time.sleep(8)
        speed.stop()
        r.check("cheater flagged by Simulation", "Simulation" in alerts(server, "GrimSpeed"), str(alerts(server, "GrimSpeed")[:5]))
        r.check("staff alert delivered in game", any("GrimSpeed" in l and "failed" in l for l in staff.lines("CHAT")))
        r.check("legitimate player never flagged (alert-only)", not alerts(server, "GrimLegit"), str(alerts(server, "GrimLegit")[:10]))
        r.check("staff (standing) never flagged", not alerts(server, "GrimStaff"), str(alerts(server, "GrimStaff")[:10]))
        server.console("grim profile GrimLegit")
        r.check("legitimate player is being judged", server.wait_for(r"Predictions: [1-9]\d* \(", 20))

        # --- Phase B: setback mode.
        config.write_text(grim_config("setback"))
        server.console("grim reload")
        r.check("reload into setback mode", server.wait_for(r"Reloaded grim-horizon.toml \(Setback mode\)", 20))
        legit_teleports = len(legit.lines("TELEPORT"))
        speed2 = bot("GrimSpeedB", "speed", 15)
        speed2.wait_for("SPAWNED", 60)
        speed2.stop()
        r.check("cheater set back", len(speed2.lines("TELEPORT")) >= 3, f"{len(speed2.lines('TELEPORT'))} teleports")
        r.check("legitimate player never set back", len(legit.lines("TELEPORT")) == legit_teleports)
        legit.stop()
        r.check("legitimate player never flagged (setback)", not alerts(server, "GrimLegit"), str(alerts(server, "GrimLegit")[:10]))

        # --- Phase C: punish mode, kicks and reconnects.
        config.write_text(grim_config("punish", kick_vl=20.0))
        server.console("grim reload")
        r.check("reload into punish mode", server.wait_for(r"Reloaded grim-horizon.toml \(Punish mode\)", 20))
        first = bot("GrimSpeedC", "speed", 30)
        r.check("cheater kicked", first.wait_for("DISCONNECTED", 40))
        first.stop()
        before = len(alerts(server, "GrimSpeedC"))
        again = bot("GrimSpeedC", "speed", 30)
        r.check("same name reconnects", again.wait_for("SPAWNED", 60))
        r.check("reconnected cheater judged and kicked again", again.wait_for("DISCONNECTED", 40) and len(alerts(server, "GrimSpeedC")) > before)
        again.stop()
        legit2 = bot("GrimLegit", "legit", 20)
        r.check("legitimate player reconnects", legit2.wait_for("SPAWNED", 60))
        legit2.stop()
        r.check("reconnected legitimate player clean", not alerts(server, "GrimLegit") and not legit2.lines("DISCONNECTED"))

        r.check("ping transactions answered", "has not answered transactions" not in server.text())
        r.check("no player left unjudged for long", "has not been judged" not in server.text(), re.findall(r".*has not been judged.*", server.text())[:3])
        server.console("grim stats")
        server.wait_for(r"block changes:", 10)
        staff.stop()
        server.console("stop")
        server.proc.wait(timeout=120)
        r.check("server stops cleanly", server.proc.returncode == 0, f"exit {server.proc.returncode}")
    except SystemExit:
        pass
    finally:
        for b in bots:
            if b.proc.poll() is None:
                b.proc.kill()
        if server.proc.poll() is None:
            server.proc.kill()

    out = ROOT / "work" / "smoke"
    out.mkdir(parents=True, exist_ok=True)
    (out / "server.log").write_text(server.text())
    for b in bots:
        (out / f"bot-{b.name}-{b.proc.pid}.log").write_text("\n".join(b.out))
    stats = [l for l in server.log if "not judged:" in l or "block changes:" in l or "µs per prediction" in l]
    for l in stats:
        print("stats:", l)
    if not args.keep:
        shutil.rmtree(scratch, ignore_errors=True)
    failed = [n for n, ok, _ in r.items if not ok]
    print(f"logs saved to {out}")
    if failed:
        print(f"LIVE SMOKE TEST FAILED ({len(failed)} of {len(r.items)}):\n  " + "\n  ".join(failed))
        sys.exit(1)
    print(f"LIVE SMOKE TEST PASSED ({len(r.items)} checks)")


if __name__ == "__main__":
    main()
