#!/usr/bin/env python3
"""Execute generated bootstraps against disposable, explicitly synthetic clients.

The installer never launches the fixture DLL/EXE bytes. --legacy-scripts-dir is
only a RED-test adapter for the previously exported bootstrap scripts.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import struct
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import unittest
import zipfile

REPO = Path(__file__).resolve().parents[1]
TEMPLATES = REPO / 'src/web/bootstrap'
PARSER = argparse.ArgumentParser()
PARSER.add_argument('--shell', choices=['bash', 'powershell', 'all'], default='all')
PARSER.add_argument('--legacy-scripts-dir', type=Path)
PARSER.add_argument('--require-powershell', action='store_true')
OPTIONS, TEST_ARGS = PARSER.parse_known_args()
SHELLS = ['bash', 'powershell'] if OPTIONS.shell == 'all' else [OPTIONS.shell]
if 'powershell' in SHELLS and not shutil.which('pwsh') and not shutil.which('powershell'):
    if OPTIONS.require_powershell:
        raise SystemExit('PowerShell is required for this verification run')
    SHELLS.remove('powershell')
    print('SKIP PowerShell execution: no pwsh/powershell available', file=sys.stderr)

REQUIRED = ['BepInEx/plugins/Fika/Fika.Core.dll',
            'BepInEx/plugins/Convoy/Convoy.dll',
            'BepInEx/plugins/Convoy/System.IO.Hashing.dll']
CONFIG = dict(server_name="Fixture ' Sanctuary", server_url='https://tarkov.example.test',
              archive_url='http://127.0.0.1:1/unreachable.zip', spt_version='4.1.6',
              tarkov_version='0.16.9.40743', required_files=REQUIRED)


def render(shell, config):
    if OPTIONS.legacy_scripts_dir:
        suffix = 'sh' if shell == 'bash' else 'ps1'
        return (OPTIONS.legacy_scripts_dir / ('bootstrap.' + suffix)).read_text()
    encoded = base64.b64encode(json.dumps(config).encode()).decode()
    if shell == 'bash':
        body = (TEMPLATES / 'client.py').read_text().replace('__QUMA_CONFIG_BASE64__', encoded)
        return (TEMPLATES / 'bootstrap.sh').read_text().replace('__QUMA_PYTHON__', body)
    return (TEMPLATES / 'bootstrap.ps1').read_text().replace('__QUMA_CONFIG_BASE64__', encoded)


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='quma-bootstrap-test-', dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / 'client space ü'
        self.root.mkdir()
        self.runtime = self.root / 'SPT_Runtime'
        (self.runtime / 'SPT_Data/configs').mkdir(parents=True)
        (self.runtime / 'SPT_Data/configs/core.json').write_text(json.dumps({'compatibleTarkovVersion': '0.16.9.40743'}))
        (self.runtime / 'SPT.Server.deps.json').write_text(json.dumps({'libraries': {'SPT.Server/4.1.6-RELEASE+fixture': {}}}))
        for relative in ['EscapeFromTarkov.exe', 'SPT_Runtime/SPT.Launcher.exe', 'SPT_Runtime/SPT.Launcher.Linux']:
            (self.root / relative).write_bytes(b'EXPLICIT SYNTHETIC CLIENT FIXTURE - NEVER EXECUTE')
        (self.root / 'BepInEx').mkdir()
        self.settings = self.runtime / 'user/Launcher/LauncherSettings.json'
        self.settings.parent.mkdir(parents=True)
        self.original = {'Servers': [{'Name': 'Other server', 'IpAddress': 'other.example.test:6969', 'ServerId': '101'}],
                         'Language': 'de', 'LinuxSettings': {'ProtonPath': 'keep-my-runner'},
                         'PreferredProfile': {'ServerId': '101', 'ProfileId': 'fixture-profile'}}
        self.settings.write_text(json.dumps(self.original))
        self.archive = self.base / 'pack.zip'
        self.write_zip()

    def write_zip(self, extra=None, omit=None):
        with zipfile.ZipFile(self.archive, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for name in REQUIRED:
                if name != omit:
                    archive.writestr(name, ('fixture bytes: ' + name).encode())
            archive.writestr('BepInEx/config/fixture.cfg', b'server default')
            archive.writestr('BepInEx/config/VisitAPI/audio/fixture.ogg', b'new resource asset')
            for name, value in (extra or {}).items():
                archive.writestr(name, value)

    def invoke(self, shell, *extra, input_text='', local_archive=True, config=None, fault=None):
        scripts = self.base / ('run-' + shell)
        scripts.mkdir(exist_ok=True)
        suffix = 'sh' if shell == 'bash' else 'ps1'
        script = scripts / ('bootstrap.' + suffix)
        source = render(shell, config or CONFIG)
        # Faults exist only in disposable generated test scripts, never production.
        if fault:
            capacity = int(fault.partition(':')[2] or '1024') if fault.startswith('capacity') else 0
            if shell == 'bash':
                injection = ("_real_read_json = read_json\ndef read_json(path):\n    if path.name == 'LauncherSettings.json':\n        raise OSError('injected final readback IO failure')\n    return _real_read_json(path)\n" if fault == 'readback' else
                             "_real_disk_usage = shutil.disk_usage\ndef _limited_space(path):\n    return _real_disk_usage(path)._replace(free=MARGIN + " + str(capacity) + ")\nshutil.disk_usage = _limited_space\n")
                source = source.replace("if __name__ == '__main__':", injection + "\nif __name__ == '__main__':")
            else:
                injection = ("$OriginalReadJson = ${function:Read-JsonFile}\nfunction Read-JsonFile { param([string]$Path)\n if ([IO.Path]::GetFileName($Path) -eq 'LauncherSettings.json') { throw [IO.IOException]::new('injected final readback IO failure') }\n & $OriginalReadJson $Path\n}\n" if fault == 'readback' else
                             "function Get-FreeSpace { param([string]$Root) return ($Margin + " + str(capacity) + ") }\n")
                prefix, main = source.rsplit('\ntry {', 1)
                source = prefix + '\n' + injection + '\ntry {' + main
        script.write_text(source, encoding='utf-8')
        environment = os.environ.copy()
        if OPTIONS.legacy_scripts_dir:
            # The RED baseline used an external native installer. Stub that
            # boundary explicitly instead of running it or accessing GitHub.
            tools = self.base / 'tools'
            tools.mkdir(exist_ok=True)
            (tools / 'wine').write_text('#!/bin/sh\nexit 0\n')
            (tools / 'wine').chmod(0o755)
            (tools / 'curl').write_text('#!/usr/bin/python3\nimport pathlib,os,shutil,sys\na=sys.argv[1:]; p=pathlib.Path(a[a.index("-o")+1]); shutil.copyfile(os.environ["FIXTURE_ARCHIVE"],p) if "mods.zip" in a[-1] else p.write_bytes(b"FAKE INSTALLER")\n')
            (tools / 'curl').chmod(0o755)
            environment['PATH'] = str(tools) + os.pathsep + environment['PATH']
            environment['FIXTURE_ARCHIVE'] = str(self.archive)
        if shell == 'bash':
            args = ['bash', str(script), '--spt-dir', str(self.root)]
            if local_archive:
                args += ['--archive', str(self.archive)]
            args += list(extra)
        else:
            runtime = shutil.which('pwsh') or shutil.which('powershell')
            args = [runtime, '-NoLogo', '-NoProfile', '-NonInteractive', '-File', str(script), '-SptDir', str(self.root)]
            if local_archive:
                args += ['-ArchivePath', str(self.archive)]
            args += list(extra)
            if OPTIONS.legacy_scripts_dir:
                wrapper = scripts / 'legacy-wrapper.ps1'
                wrapper.write_text('''function Invoke-WebRequest { param($Uri,$OutFile,[switch]$UseBasicParsing)
if ($Uri -like '*Fika-Installer*') { [IO.File]::WriteAllText($OutFile,"#!/bin/sh`nexit 0`n"); & /usr/bin/chmod +x $OutFile }
else { Copy-Item $env:FIXTURE_ARCHIVE $OutFile } }
& $args[0]
''')
                args = [runtime, '-NoLogo', '-NoProfile', '-NonInteractive', '-File', str(wrapper), str(script)]
        if os.name != 'nt' and shutil.which('bwrap'):
            scratch_tmp = self.base / 'sandbox-tmp'
            scratch_tmp.mkdir(exist_ok=True)
            scratch_home = self.base / 'sandbox-home'
            scratch_home.mkdir(exist_ok=True)
            args = ['bwrap', '--ro-bind', '/', '/', '--bind', str(self.base), str(self.base),
                    '--bind', str(scratch_tmp), '/tmp', '--proc', '/proc', '--dev', '/dev',
                    '--die-with-parent', '--new-session', '--setenv', 'HOME', str(scratch_home),
                    '--setenv', 'TMPDIR', '/tmp', '--setenv', 'TEMP', '/tmp',
                    '--chdir', str(self.root), '--'] + args
        result = subprocess.run(args, cwd=self.root, env=environment, input=input_text,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=40)
        return result

    def yes(self, shell):
        return '--yes' if shell == 'bash' else '-Yes'

    def assert_failure(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertNotIn('=== Setup Complete ===', result.stdout)

    def test_current_launcher_merge_and_rerun_preserve_identity(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                self.settings.write_text(json.dumps(self.original))
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                saved = json.loads(self.settings.read_text(encoding='utf-8-sig'))
                servers = [s for s in saved['Servers'] if s['IpAddress'] == 'tarkov.example.test']
                self.assertEqual(len(servers), 1, result.stdout)
                self.assertTrue(servers[0]['ServerId'].isdigit())
                self.assertNotEqual(servers[0]['ServerId'], '1721162719')
                self.assertEqual(saved['Servers'][0], self.original['Servers'][0])
                self.assertEqual(saved['PreferredProfile'], self.original['PreferredProfile'])
                self.assertEqual(saved['LinuxSettings'], self.original['LinuxSettings'])
                before = self.settings.read_bytes()
                again = self.invoke(shell, self.yes(shell))
                self.assertEqual(again.returncode, 0, again.stdout)
                self.assertEqual(self.settings.read_bytes(), before)

    def test_fresh_launcher_settings_are_created(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                self.settings.unlink(missing_ok=True)
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertEqual(json.loads(self.settings.read_text(encoding='utf-8-sig'))['Servers'][0]['IpAddress'], 'tarkov.example.test')

    def test_config_preserved_and_changed_plugins_and_assets_backed_up(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                config = self.root / 'BepInEx/config/fixture.cfg'
                config.parent.mkdir(parents=True, exist_ok=True)
                config.write_bytes(b'my customization')
                plugin = self.root / REQUIRED[0]
                plugin.parent.mkdir(parents=True, exist_ok=True)
                plugin.write_bytes(b'old plugin')
                asset = self.root / 'BepInEx/config/VisitAPI/audio/fixture.ogg'
                asset.parent.mkdir(parents=True, exist_ok=True)
                asset.write_bytes(b'old asset')
                installer = self.root / 'Fika-Installer.exe'
                installer.write_bytes(b'personal existing installer')
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertEqual(config.read_bytes(), b'my customization')
                self.assertEqual(installer.read_bytes(), b'personal existing installer')
                self.assertEqual(plugin.read_bytes(), ('fixture bytes: ' + REQUIRED[0]).encode())
                self.assertEqual(asset.read_bytes(), b'new resource asset')
                backup = self.root / '.quma-bootstrap-backups'
                self.assertTrue(any(p.read_bytes() == b'old plugin' for p in backup.rglob('Fika.Core.dll')))
                self.assertTrue(any(p.read_bytes() == b'old asset' for p in backup.rglob('fixture.ogg')))

    def test_dry_run_does_not_change_client(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                before = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
                result = self.invoke(shell, '--dry-run' if shell == 'bash' else '-DryRun')
                self.assertEqual(result.returncode, 0, result.stdout)
                after = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
                self.assertEqual(before, after)
                self.assertNotIn('=== Setup Complete ===', result.stdout)

    def test_confirmation_eof_is_failure_without_mutation(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                before = self.settings.read_bytes()
                result = self.invoke(shell)
                self.assert_failure(result)
                self.assertEqual(self.settings.read_bytes(), before)

    def test_incompatible_spt_fails_without_mutation(self):
        path = self.runtime / 'SPT.Server.deps.json'
        path.write_text(json.dumps({'libraries': {'SPT.Server/4.0.13-RELEASE': {}}}))
        for shell in SHELLS:
            with self.subTest(shell=shell):
                self.assert_failure(self.invoke(shell, self.yes(shell)))
        self.assertFalse((self.root / REQUIRED[0]).exists())

    def test_duplicate_launcher_properties_fail_before_install(self):
        base = json.dumps(self.original)[:-1]
        cases = {
            'servers': base + ', "Servers": []}',
            'escaped_servers': base + r', "Ser\u0076ers": []}',
            'case_servers': base + ', "servers": []}',
            'runner': json.dumps(self.original).replace(json.dumps(self.original['LinuxSettings']), '{"ProtonPath":"keep", "ProtonPath":"lost"}'),
            'unknown': base + ', "Extra": {"Unknown":1, "Unknown":2}}',
            'array_object': base + ', "Extra": [[{"Name":"keep", "Name":"lost"}]]}',
            'escaped_nested': base + r', "Extra": [{"Na\u006de":1, "Name":2}]}',
            'case_nested': base + ', "Extra": [{"Name":1, "name":2}]}',
            'comment_scope': base + ', /* } [ */ "Servers": []}',
            'single_quoted_key': base + ", 'Servers': []}",
            'unquoted_key': base + ', Servers: []}',
            'nbsp_before_colon': base + ', "Servers"\u00a0: []}',
        }
        for shell in SHELLS:
            for name, text in cases.items():
                with self.subTest(shell=shell, case=name):
                    self.settings.write_bytes(b'\xef\xbb\xbf' + text.encode('utf-8'))
                    before = {p.relative_to(self.root): p.read_bytes() if p.is_file() else None
                              for p in self.root.rglob('*')}
                    result = self.invoke(shell, self.yes(shell))
                    # Verify preservation even if a broken implementation exits successfully.
                    after = {p.relative_to(self.root): p.read_bytes() if p.is_file() else None
                             for p in self.root.rglob('*')}
                    self.assertEqual(before, after, result.stdout)
                    self.assert_failure(result)
                    if name not in ('comment_scope', 'single_quoted_key', 'unquoted_key', 'nbsp_before_colon'):
                        self.assertIn('duplicate', result.stdout.lower())
                    self.assertFalse((self.root / '.quma-bootstrap-backups').exists())

    def test_distinct_launcher_objects_and_escaped_strings_are_preserved(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                expected = dict(self.original)
                expected['Extra'] = [{'Name': 'quote " slash \\ newline\n ü', 'Nested': [{'Name': 'one'}]},
                                     {'Name': 'two', 'Nested': [[], {'Name': 'three'}]}]
                text = json.dumps(expected).replace('"Name"', r'"Na\u006de"')
                self.settings.write_bytes(b'\xef\xbb\xbf' + text.encode('utf-8'))
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                saved = json.loads(self.settings.read_text(encoding='utf-8-sig'))
                for key in ('Extra', 'PreferredProfile', 'LinuxSettings'):
                    self.assertEqual(saved[key], expected[key])
                self.assertEqual(saved['Servers'][0], expected['Servers'][0])

    def test_malformed_launcher_settings_fail_before_install(self):
        self.settings.write_text('{invalid json')
        for shell in SHELLS:
            with self.subTest(shell=shell):
                self.assert_failure(self.invoke(shell, self.yes(shell)))
        self.assertFalse((self.root / REQUIRED[0]).exists())
        self.assertEqual(self.settings.read_text(), '{invalid json')

    def test_unsafe_archive_names_and_case_collisions_fail(self):
        for name in ['../escape.txt', '/absolute.txt', 'C:/escape.txt', 'BepInEx/../escape.txt',
                     'BepInEx/plugins/data:stream', 'BepInEx/plugins/NUL.txt',
                     'SPT_Runtime/user/profiles/fixture.json', 'BepInEx/plugins/fika/fika.core.dll']:
            self.write_zip(extra={name: b'UNSAFE FIXTURE'})
            for shell in SHELLS:
                with self.subTest(shell=shell, name=name):
                    self.assert_failure(self.invoke(shell, self.yes(shell)))
                    self.assertFalse((self.root / REQUIRED[0]).exists())
        self.assertFalse((self.base / 'escape.txt').exists())

    @unittest.skipIf(os.name == 'nt', 'POSIX symlink fixture; native reparse checks are separate')
    def test_archive_symlink_and_existing_destination_symlink_fail(self):
        link = zipfile.ZipInfo('BepInEx/plugins/evil')
        link.create_system = 3
        link.external_attr = (stat.S_IFLNK | 0o777) << 16
        with zipfile.ZipFile(self.archive, 'a') as archive:
            archive.writestr(link, '../outside')
        for shell in SHELLS:
            self.assert_failure(self.invoke(shell, self.yes(shell)))
        self.write_zip()
        outside = self.base / 'outside'
        outside.mkdir()
        (self.root / 'BepInEx/plugins').symlink_to(outside, target_is_directory=True)
        for shell in SHELLS:
            self.assert_failure(self.invoke(shell, self.yes(shell)))
        self.assertEqual(list(outside.iterdir()), [])

    def test_incomplete_or_corrupt_archive_fails_without_install(self):
        self.write_zip(omit=REQUIRED[0])
        for shell in SHELLS:
            self.assert_failure(self.invoke(shell, self.yes(shell)))
        self.archive.write_bytes(b'not a ZIP')
        for shell in SHELLS:
            self.assert_failure(self.invoke(shell, self.yes(shell)))
        self.assertFalse((self.root / REQUIRED[1]).exists())

    def test_valid_archive_with_wrong_crc_is_rejected_before_install(self):
        data = bytearray(self.archive.read_bytes())
        central = data.index(b'PK\x01\x02')
        crc = struct.unpack_from('<I', data, central + 16)[0]
        struct.pack_into('<I', data, central + 16, crc ^ 1)
        self.archive.write_bytes(data)
        for shell in SHELLS:
            self.assert_failure(self.invoke(shell, self.yes(shell)))
            self.assertFalse((self.root / REQUIRED[0]).exists())

    def test_zip64_end_records_are_supported(self):
        data = self.archive.read_bytes()
        offset = data.rindex(b'PK\x05\x06')
        end = bytearray(data[offset:])
        count, size, start = struct.unpack_from('<HII', end, 10)
        record = struct.pack('<4sQHHIIQQQQ', b'PK\x06\x06', 44, 45, 45, 0, 0, count, count, size, start)
        locator = struct.pack('<4sIQI', b'PK\x06\x07', 0, offset, 1)
        struct.pack_into('<HHII', end, 8, 65535, 65535, 4294967295, 4294967295)
        self.archive.write_bytes(data[:offset] + record + locator + end)
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertEqual((self.root / REQUIRED[0]).read_bytes(), ('fixture bytes: ' + REQUIRED[0]).encode())

    @unittest.skipIf(os.name == 'nt', 'POSIX permission failure fixture')
    def test_mid_apply_io_failure_restores_replaced_files(self):
        first = self.root / REQUIRED[0]
        first.parent.mkdir(parents=True)
        first.write_bytes(b'original plugin')
        blocked = self.root / 'BepInEx/plugins/Convoy'
        blocked.mkdir()
        blocked.chmod(0o555)
        self.addCleanup(lambda: blocked.chmod(0o755))
        before = self.settings.read_bytes()
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, self.yes(shell))
                self.assert_failure(result)
                self.assertEqual(first.read_bytes(), b'original plugin')
                self.assertEqual(self.settings.read_bytes(), before)
                manifests = list((self.root / '.quma-bootstrap-backups').glob('*/manifest.json'))
                self.assertTrue(manifests)
                self.assertTrue(all(json.loads(p.read_text())['status'] == 'rolled_back' for p in manifests))

    def test_streamed_http_download_and_explicit_confirmation(self):
        data = self.archive.read_bytes()
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, format, *args):
                pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        config = dict(CONFIG, archive_url='http://127.0.0.1:' + str(server.server_port) + '/pack.zip')
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, input_text='y\n', local_archive=False, config=config)
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertIn('=== Setup Complete ===', result.stdout)

    def test_launcher_edited_during_download_is_preserved_and_aborts(self):
        data = self.archive.read_bytes()
        settings = self.settings
        external = dict(self.original, Language='external-edit')
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                settings.write_text(json.dumps(external))
                self.send_response(200)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, format, *args):
                pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        config = dict(CONFIG, archive_url='http://127.0.0.1:' + str(server.server_port) + '/pack.zip')
        for shell in SHELLS:
            with self.subTest(shell=shell):
                self.settings.write_text(json.dumps(self.original))
                result = self.invoke(shell, self.yes(shell), local_archive=False, config=config)
                self.assert_failure(result)
                self.assertEqual(json.loads(self.settings.read_text())['Language'], 'external-edit')
                self.assertFalse((self.root / REQUIRED[0]).exists())

    def test_instance_source_distribution_payload_is_supported(self):
        self.write_zip(extra={'source/GamePanelHUD-src.zip': b'fixture source distribution'})
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertEqual((self.root / 'source/GamePanelHUD-src.zip').read_bytes(), b'fixture source distribution')

    def test_ambiguous_or_wrongly_typed_server_entries_fail_closed(self):
        cases = [
            {'Servers': [{'Name': 'first', 'IpAddress': 'tarkov.example.test', 'ServerId': '101'},
                         {'Name': 'second', 'IpAddress': 'https://tarkov.example.test/', 'ServerId': '102'}]},
            {'Servers': [{'Name': 17, 'IpAddress': 'tarkov.example.test', 'ServerId': 123}]},
        ]
        for settings in cases:
            for shell in SHELLS:
                with self.subTest(shell=shell, settings=settings):
                    before = json.dumps(settings).encode()
                    self.settings.write_bytes(before)
                    result = self.invoke(shell, self.yes(shell))
                    self.assert_failure(result)
                    self.assertEqual(self.settings.read_bytes(), before)
                    self.assertFalse((self.root / REQUIRED[0]).exists())

    def test_short_alias_archive_names_are_rejected(self):
        for name in ['ESCAPE~1.EXE', 'BepInEx/PLUGIN~1/payload.dll']:
            self.write_zip(extra={name: b'alias payload'})
            for shell in SHELLS:
                with self.subTest(shell=shell, name=name):
                    self.assert_failure(self.invoke(shell, self.yes(shell)))
                    self.assertFalse((self.root / REQUIRED[0]).exists())

    @unittest.skipUnless(os.name == 'nt' and 'powershell' in SHELLS, 'requires native Windows NTFS short names and PowerShell')
    def test_native_custom_short_alias_is_rejected(self):
        import ctypes
        target = self.root / 'EscapeFromTarkov.exe'
        kernel = getattr(ctypes, 'WinDLL')('kernel32', use_last_error=True)
        kernel.SetFileShortNameW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p]
        handle = kernel.CreateFileW(str(target), 0x10000 | 0x100, 7, None, 3, 0, None)
        if handle == ctypes.c_void_p(-1).value:
            self.skipTest('cannot open fixture to set NTFS short name')
        try:
            if not kernel.SetFileShortNameW(handle, 'OWNED.EXE'):
                self.skipTest('volume/policy does not support setting NTFS short names')
        finally:
            kernel.CloseHandle.argtypes = [ctypes.c_void_p]
            kernel.CloseHandle(handle)
        before = target.read_bytes()
        self.assertEqual((self.root / 'OWNED.EXE').read_bytes(), before)
        self.write_zip(extra={'OWNED.EXE': b'must not replace owned game'})
        self.assert_failure(self.invoke('powershell', '-Yes'))
        self.assertEqual(target.read_bytes(), before)
        self.assertFalse((self.root / REQUIRED[0]).exists())

    def test_final_readback_io_failure_rolls_back_transaction(self):
        first = self.root / REQUIRED[0]
        first.parent.mkdir(parents=True)
        first.write_bytes(b'original plugin')
        before = self.settings.read_bytes()
        for shell in SHELLS:
            with self.subTest(shell=shell):
                first.write_bytes(b'original plugin')
                self.settings.write_bytes(before)
                for name in REQUIRED[1:] + ['BepInEx/config/fixture.cfg', 'BepInEx/config/VisitAPI/audio/fixture.ogg']:
                    (self.root / name).unlink(missing_ok=True)
                shutil.rmtree(self.root / '.quma-bootstrap-backups', ignore_errors=True)
                result = self.invoke(shell, self.yes(shell), fault='readback')
                self.assert_failure(result)
                self.assertIn('injected final readback IO failure', result.stdout)
                self.assertEqual(first.read_bytes(), b'original plugin')
                self.assertEqual(self.settings.read_bytes(), before)
                self.assertFalse((self.root / REQUIRED[1]).exists())
                manifests = list((self.root / '.quma-bootstrap-backups').glob('*/manifest.json'))
                self.assertTrue(manifests)
                self.assertTrue(all(json.loads(p.read_text())['status'] == 'rolled_back' for p in manifests))

    def test_peak_capacity_refuses_before_client_writes(self):
        with zipfile.ZipFile(self.archive) as archive:
            expanded = sum(i.file_size for i in archive.infolist())
        self.assertLess(expanded, 1024)  # Old E+B+margin gate accepts this free space.
        before = self.settings.read_bytes()
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, self.yes(shell), fault='capacity')
                self.assert_failure(result)
                self.assertIn('space', result.stdout.lower())
                self.assertEqual(self.settings.read_bytes(), before)
                self.assertFalse((self.root / REQUIRED[0]).exists())
                self.assertFalse((self.root / '.quma-bootstrap-backups').exists())

    def test_peak_capacity_counts_staged_install_and_backup_copies(self):
        self.write_zip(extra={'BepInEx/plugins/large.dll': b'x' * 65536})
        plugin = self.root / REQUIRED[0]
        plugin.parent.mkdir(parents=True)
        plugin.write_bytes(b'old' * 32768)
        with zipfile.ZipFile(self.archive) as archive:
            expanded = sum(i.file_size for i in archive.infolist())
        old_gate = expanded + plugin.stat().st_size
        before = self.settings.read_bytes()
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, self.yes(shell), fault='capacity:' + str(old_gate + 1))
                self.assert_failure(result)
                self.assertIn('space', result.stdout.lower())
                self.assertEqual(self.settings.read_bytes(), before)
                self.assertEqual(plugin.read_bytes(), b'old' * 32768)
                self.assertFalse((self.root / REQUIRED[1]).exists())
                self.assertFalse((self.root / '.quma-bootstrap-backups').exists())

    def test_idempotent_transaction_still_reads_back_settings(self):
        for shell in SHELLS:
            with self.subTest(shell=shell):
                result = self.invoke(shell, self.yes(shell))
                self.assertEqual(result.returncode, 0, result.stdout)
                before = self.settings.read_bytes()
                manifests = list((self.root / '.quma-bootstrap-backups').glob('*/manifest.json'))
                result = self.invoke(shell, self.yes(shell), fault='readback')
                self.assert_failure(result)
                self.assertIn('injected final readback IO failure', result.stdout)
                self.assertEqual(self.settings.read_bytes(), before)
                self.assertEqual(list((self.root / '.quma-bootstrap-backups').glob('*/manifest.json')), manifests)

    def test_failed_download_is_not_success(self):
        for shell in SHELLS:
            self.assert_failure(self.invoke(shell, self.yes(shell), local_archive=False))
            self.assertFalse((self.root / REQUIRED[0]).exists())


if __name__ == '__main__':
    unittest.main(argv=[sys.argv[0]] + TEST_ARGS, verbosity=2)
