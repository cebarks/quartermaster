#!/usr/bin/env python3
"""Verify headless Xvfb ownership with real X servers and synthetic Wine children."""
import argparse
import os
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import re
import socket
import struct
import subprocess as sp
import tempfile
import time
import uuid


def podman(*args, check=True):
    return sp.run(['podman', *args], check=check, capture_output=True, text=True, timeout=25)


def read_exact(sock, length):
    data = b''
    while len(data) < length:
        chunk = sock.recv(length - len(data))
        if not chunk:
            raise RuntimeError('X server closed the connection')
        data += chunk
    return data


def connect(display):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.settimeout(3)
        sock.connect(f'\0/tmp/.X11-unix/X{display}')
        sock.sendall(struct.pack('<BBHHHHH', ord('l'), 0, 11, 0, 0, 0, 0))
        status, _, _, _, extra = struct.unpack('<BBHHH', read_exact(sock, 8))
        assert status == 1, status
        read_exact(sock, 4 * extra)
        return sock
    except BaseException:
        sock.close()
        raise


def ping(sock, sequence):
    # X11 GetInputFocus round trip on the same connection.
    sock.sendall(struct.pack('<BBH', 43, 0, 1))
    response = read_exact(sock, 32)
    assert response[0] == 1 and struct.unpack_from('<H', response, 2)[0] == sequence


def script(path, text):
    path.write_text('#!/bin/sh\nset -eu\n' + text)
    path.chmod(0o700)


def wait_file(path, timeout=5):
    deadline = time.monotonic() + timeout
    while not path.exists() and time.monotonic() < deadline:
        time.sleep(0.02)
    assert path.exists(), f'timed out waiting for {path.name}'


def run(image, entrypoint, tmp):
    run_id = 'quma-xvfb-test-' + uuid.uuid4().hex[:12]
    containers = []
    wine = tmp / 'wine-bin'
    wine.mkdir()
    script(wine / 'wineboot', '''
        printf '%s\\n' "$DISPLAY" > /opt/tarkov/wineboot-display
        if [ "${BOOT_MODE:-}" = stalled ]; then
            trap 'echo TERM > /opt/tarkov/boot-term; sleep 0.4; test -S /tmp/.X11-unix/X${DISPLAY#:}; echo clean > /opt/tarkov/boot-clean; exit 0' TERM
            echo ready > /opt/tarkov/boot-ready
            while :; do sleep 0.1; done
        fi
    ''')
    script(wine / 'wineserver', 'exit 0\n')
    script(wine / 'wine', '''
        printf '%s\\n' "$DISPLAY" > /opt/tarkov/wine-display
        # Keep a wrapper PID distinct from the real long-lived child.
        trap 'wait; exit 0' TERM
        /bin/sh /opt/wine-cachyos/bin/game &
        wait
    ''')
    script(wine / 'game', '''
        if [ "${GAME_MODE:-}" = stubborn ]; then
            trap 'echo TERM > /opt/tarkov/game-term' TERM
        else
            trap 'echo TERM > /opt/tarkov/game-term; sleep 0.4; test -S /tmp/.X11-unix/X${DISPLAY#:}; echo clean > /opt/tarkov/game-clean; exit 0' TERM
        fi
        echo ready > /opt/tarkov/game-ready
        while :; do sleep 0.1 || :; done
    ''')
    fault = tmp / 'fault-bin'
    fault.mkdir()
    script(fault / 'Xvfb', '''
        case "$X_TEST_MODE" in
            dead) exit 22 ;;
            stalled) exec sleep 30 ;;
            malformed) printf 'not-a-display\\n' >&3; exec sleep 30 ;;
        esac
    ''')

    def start(suffix, *flags):
        name = run_id + '-' + suffix
        game = tmp / suffix
        (game / 'BepInEx').mkdir(parents=True)
        (game / 'BepInEx/LogOutput.log').touch()
        (game / 'user.reg').touch()
        containers.append(name)
        podman('run', '-d', '--pull=never', '--network=host', '--name', name,
               '--label', f'quma.xvfb-test={run_id}',
               '--timeout', '90',
               '--security-opt', 'label=disable', '-e', 'PROFILE_ID=fixture',
               '-e', 'WINEPREFIX=/opt/tarkov', '-v', f'{game}:/opt/tarkov:rw',
               '-v', f'{wine}:/opt/wine-cachyos/bin:ro', *flags,
               *(['-v', f'{entrypoint}:/entrypoint.sh:ro'] if entrypoint else []), image)
        return name, game

    def wait_display(name, game):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                display = (game / 'wine-display').read_text().strip()
            except FileNotFoundError:
                time.sleep(0.1)
                continue
            assert re.fullmatch(r':[0-9]+', display), display
            assert (game / 'wineboot-display').read_text().strip() == display
            output = podman('logs', name).stdout
            assert f'Xvfb ready on {display}' in output, 'Wine launched without owned Xvfb readiness'
            return int(display[1:])
        raise RuntimeError(f'{name} did not start Wine: {podman("logs", name).stdout}')

    try:
        with ThreadPoolExecutor(max_workers=2) as pool:
            a, b = list(pool.map(start, ('a', 'b')))
        da, db = wait_display(*a), wait_display(*b)
        assert da != db, f'both entries use display :{da}'
        with connect(da) as xa, connect(db) as xb:
            ping(xa, 1)
            ping(xb, 1)
            wait_file(a[1] / 'game-ready')
            with ThreadPoolExecutor(max_workers=1) as pool:
                stopped = pool.submit(podman, 'stop', '--time', '8', a[0])
                wait_file(a[1] / 'game-term')
                ping(xa, 2)  # Real round trip while Wine is cleaning up.
                ping(xb, 2)
                stopped.result()
            assert podman('inspect', '--format', '{{.State.ExitCode}}', a[0]).stdout.strip() == '143', 'forced kill or incorrect exit'
            assert (a[1] / 'game-term').exists(), 'real game child did not receive TERM'
            assert (a[1] / 'game-clean').exists(), 'game cleanup did not finish with X alive'
            logs = podman('logs', a[0])
            assert 'sending SIGKILL' not in logs.stdout + logs.stderr, 'game required forced kill'
            assert xa.recv(1) == b'', 'A still has a display after stopping A'
            ping(xb, 3)
        print(f'PASS: simultaneous displays :{da} and :{db}; stopping A leaves B connected')

        def verify_fault(mode):
            name, game = start(mode, '-e', f'X_TEST_MODE={mode}',
                               '-e', 'PATH=/fault:/usr/bin:/bin', '-v', f'{fault}:/fault:ro')
            result = podman('wait', name)
            assert int(result.stdout.strip()) != 0, mode
            assert not (game / 'wineboot-display').exists(), mode
            assert not (game / 'wine-display').exists(), mode
            logs = podman('logs', name)
            expected = ('ERROR: Xvfb exited before reporting its display' if mode == 'dead'
                        else 'ERROR: Xvfb did not report a valid display')
            assert expected in logs.stdout + logs.stderr, logs
            return mode

        with ThreadPoolExecutor(max_workers=3) as pool:
            for mode in pool.map(verify_fault, ('dead', 'stalled', 'malformed')):
                print(f'PASS: {mode} Xvfb fails before Wine')

        name, game = start('boot-term', '-e', 'BOOT_MODE=stalled')
        deadline = time.monotonic() + 10
        while not (game / 'boot-ready').exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert (game / 'boot-ready').exists(), 'wineboot never started'
        display = int((game / 'wineboot-display').read_text().strip()[1:])
        with connect(display) as xboot, ThreadPoolExecutor(max_workers=1) as pool:
            stopped = pool.submit(podman, 'stop', '--time', '8', name)
            wait_file(game / 'boot-term')
            ping(xboot, 1)
            stopped.result()
        assert podman('inspect', '--format', '{{.State.ExitCode}}', name).stdout.strip() == '143'
        assert (game / 'boot-term').exists(), 'stalled wineboot missed TERM'
        assert (game / 'boot-clean').exists(), 'wineboot cleanup lost X or was killed'
        logs = podman('logs', name)
        assert 'sending SIGKILL' not in logs.stdout + logs.stderr, 'wineboot required forced kill'
        assert not (game / 'wine-display').exists(), 'EFT launched after TERM'
        print('PASS: PID1 TERM during stalled wineboot completes cleanup with X alive')

        name, game = start('game-stubborn', '-e', 'GAME_MODE=stubborn')
        display = wait_display(name, game)
        wait_file(game / 'game-ready')
        with connect(display) as xgame, ThreadPoolExecutor(max_workers=1) as pool:
            stopped = pool.submit(podman, 'stop', '--time', '8', name)
            wait_file(game / 'game-term')
            ping(xgame, 1)
            stopped.result()
            assert xgame.recv(1) == b'', 'Display survived final cleanup'
        assert podman('inspect', '--format', '{{.State.ExitCode}}', name).stdout.strip() == '143', 'container required external SIGKILL'
        assert not (game / 'game-clean').exists(), 'stubborn child unexpectedly exited cleanly'
        logs = podman('logs', name)
        assert 'exceeded shutdown grace; sending SIGKILL' in logs.stdout + logs.stderr
        assert 'entrypoint exited (code 143)' in logs.stdout
        print('PASS: stubborn Wine group is killed after grace without external PID1 escalation')

        # Hold the actual asynchronous child before it can create its session.
        # Ignoring TERM proves cleanup has a bounded direct-child KILL fallback.
        delayed = tmp / 'delayed-bin'
        delayed.mkdir()
        script(delayed / 'setsid', '''
            case "$1:$DELAY_STAGE" in
                */wineboot:boot|*/wine:eft)
                    trap '' TERM
                    printf '%s\\n' "$$" > /opt/tarkov/pre-session-pid
                    echo "$DISPLAY" > /opt/tarkov/pre-session-display
                    while [ ! -f /opt/tarkov/release-session ]; do sleep 0.1; done
                    ;;
            esac
            exec /usr/bin/setsid "$@"
        ''')
        launch_failures = []
        for stage in ('boot', 'eft'):
            name, game = start('pre-session-' + stage,
                               '-e', f'DELAY_STAGE={stage}',
                               '-e', 'PATH=/delayed:/usr/bin:/bin',
                               '-v', f'{delayed}:/delayed:ro')
            wait_file(game / 'pre-session-pid', timeout=15)
            pid = (game / 'pre-session-pid').read_text().strip()
            # Verify this is the real missing-PGID window, not fixture startup failure.
            stat = podman('exec', name, 'cat', f'/proc/{pid}/stat').stdout
            fields = stat.rsplit(')', 1)[1].split()
            state = [pid, *fields[1:4]]  # PID, PPID, PGID, SID
            assert state[0] == pid and state[1] == '1' and state[2] != pid, state
            (game / 'pre-session-state').write_text(' '.join(state))
            display = int((game / 'pre-session-display').read_text().strip()[1:])
            with connect(display) as xlaunch, connect(db) as xsibling:
                began = time.monotonic()
                # TERM only: no podman stop timeout can hide an unbounded wait.
                podman('kill', '--signal', 'TERM', name)
                time.sleep(0.4)
                ping(xlaunch, 1)
                ping(xsibling, 1)
                deadline = began + 7
                while time.monotonic() < deadline:
                    if podman('inspect', '--format', '{{.State.Running}}', name).stdout.strip() == 'false':
                        break
                    time.sleep(0.1)
                running = podman('inspect', '--format', '{{.State.Running}}', name).stdout.strip()
                elapsed = time.monotonic() - began
                (game / 'shutdown-result').write_text(f'running={running} elapsed={elapsed:.3f}\n')
                if running == 'true':
                    launch_failures.append(f'{stage}: PID1 still running after {elapsed:.2f}s with pre-session child')
                    print('FAIL: ' + launch_failures[-1], flush=True)
                    continue
                assert podman('inspect', '--format', '{{.State.ExitCode}}', name).stdout.strip() == '143'
                assert not (game / 'wine-display').exists(), 'EFT launched after cancellation'
                logs = podman('logs', name)
                assert 'exceeded shutdown grace; sending SIGKILL' in logs.stdout + logs.stderr
                ping(xsibling, 2)
                print(f'PASS: TERM before {stage} setsid exits in {elapsed:.2f}s without external KILL; X survives grace and sibling unaffected', flush=True)
        assert not launch_failures, '; '.join(launch_failures)

        # Kill precisely the Xvfb owned by this container during prefix update.
        script(wine / 'wineboot', '''
            printf '%s\\n' "$DISPLAY" > /opt/tarkov/wineboot-display
            for p in /proc/[0-9]*; do
                IFS= read -r comm < "$p/comm" || continue
                [ "$comm" != Xvfb ] || kill -TERM "${p##*/}"
            done
            sleep 0.3
        ''')
        name, game = start('boot-x-death')
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            assert not (game / 'wine-display').exists(), 'EFT launched after Xvfb died during boot'
            if podman('inspect', '--format', '{{.State.Running}}', name).stdout.strip() == 'false':
                break
            time.sleep(0.1)
        assert int(podman('wait', name).stdout.strip()) != 0
        assert not (game / 'wine-display').exists(), 'EFT launched after Xvfb died during boot'
        logs = podman('logs', name)
        assert 'ERROR: Xvfb exited before EFT launch' in logs.stdout + logs.stderr
        print('PASS: Xvfb death during wineboot prevents EFT launch')
    finally:
        for name in containers:
            owned = podman('inspect', '--format', '{{index .Config.Labels "quma.xvfb-test"}}', name, check=False)
            if owned.returncode == 0 and owned.stdout.strip() == run_id:
                logs = podman('logs', name, check=False)
                (tmp / f'{name}.log').write_text(logs.stdout + logs.stderr)
                (tmp / f'{name}.inspect.json').write_text(podman('inspect', name).stdout)
                podman('rm', '-f', '--time', '3', name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image')
    parser.add_argument('--entrypoint', type=lambda path: Path(path).resolve(strict=True))
    parser.add_argument('--evidence-dir', type=Path,
                        help='Keep fixture files, container logs and inspect output here')
    args = parser.parse_args()
    if args.evidence_dir:
        args.evidence_dir.mkdir(parents=True, exist_ok=False)
        run(args.image, args.entrypoint, args.evidence_dir.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='quma-xvfb-test-', dir=os.environ.get('TMPDIR')) as tmp:
            run(args.image, args.entrypoint, Path(tmp))


if __name__ == '__main__':
    main()
