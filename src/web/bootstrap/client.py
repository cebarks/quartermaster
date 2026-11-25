#!/usr/bin/env python3
"""Instance-matched client bootstrap. Embedded in the downloadable Bash script."""
import argparse
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import sys
import tempfile
import time
from urllib.parse import urlsplit
from urllib.request import Request, urlopen
import uuid
import zipfile

CONFIG = json.loads(base64.b64decode('__QUMA_CONFIG_BASE64__'))
CONFIG_SUFFIXES = {'.cfg', '.ini', '.json', '.jsonc', '.yaml', '.yml', '.toml', '.xml'}
RESERVED = re.compile(r'^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)', re.I)
MARGIN = 16 * 1024 * 1024


def safe_target(root, relative, directory=False):
    """Reject links and non-directory ancestors before accessing a target."""
    current = root
    parts = Path(relative).parts
    for index, part in enumerate(parts):
        current = current / part
        try:
            mode = current.lstat().st_mode
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(mode):
            raise ValueError('Destination contains a symbolic link: ' + str(current))
        need_directory = directory or index < len(parts) - 1
        if need_directory and not stat.S_ISDIR(mode):
            raise ValueError('Destination ancestor is not a directory: ' + str(current))
        if not need_directory and not stat.S_ISREG(mode):
            raise ValueError('Destination is not a regular file: ' + str(current))
    return current


def read_json(path):
    with path.open(encoding='utf-8-sig') as stream:
        return json.load(stream)


def server_address(url):
    parsed = urlsplit(url if '://' in url else 'https://' + url)
    if parsed.scheme not in ('https', 'http') or not parsed.hostname:
        raise ValueError('Invalid server address')
    if parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ('', '/'):
        raise ValueError('Server address must contain only a host and optional port')
    return parsed.netloc.lower()


def prepare_client(root):
    if not root.is_dir():
        raise ValueError('SPT client directory does not exist')
    safe_target(root, 'EscapeFromTarkov.exe')
    if not (root / 'EscapeFromTarkov.exe').is_file():
        raise ValueError('EscapeFromTarkov.exe not found; an existing owned SPT client is required')
    safe_target(root, 'BepInEx', directory=True)
    if not (root / 'BepInEx').is_dir():
        raise ValueError('BepInEx/ not found; install the matching SPT client first')
    runtimes = [name for name in ('SPT_Runtime', 'SPT')
                if (root / name / 'SPT_Data/configs/core.json').is_file()]
    if len(runtimes) != 1:
        raise ValueError('Expected exactly one unambiguous SPT runtime')
    runtime = runtimes[0]
    for relative in (runtime + '/SPT_Data/configs/core.json', runtime + '/SPT.Server.deps.json'):
        safe_target(root, relative)
    deps = read_json(root / runtime / 'SPT.Server.deps.json')
    versions = [name.split('/', 1)[1].split('-', 1)[0]
                for name in deps.get('libraries', {}) if name.startswith('SPT.Server/')]
    if versions != [CONFIG['spt_version']]:
        raise ValueError('Client SPT version does not match this instance (' + CONFIG['spt_version'] + ')')
    core = read_json(root / runtime / 'SPT_Data/configs/core.json')
    version = core.get('compatibleTarkovVersion', core.get('compatible_tarkov_version'))
    if version != CONFIG['tarkov_version']:
        raise ValueError('Client EFT compatibility metadata does not match ' + CONFIG['tarkov_version'])
    launchers = [runtime + '/SPT.Launcher.Linux', runtime + '/SPT.Launcher.exe']
    if not any((root / path).is_file() for path in launchers):
        raise ValueError('Current SPT launcher not found')
    for relative in launchers:
        safe_target(root, relative)
    return runtime


def unique_launcher_properties(pairs):
    # Per-object pairs retain duplicates, including JSON-escaped spellings.
    # Reject case collisions as well: PowerShell's object converter loses them.
    seen = set()
    for name, _ in pairs:
        folded = name.upper()
        if folded in seen:
            raise ValueError('Duplicate launcher JSON property: ' + name)
        seen.add(folded)
    return dict(pairs)


def launcher_update(root, runtime):
    relative = runtime + '/user/Launcher/LauncherSettings.json'
    path = safe_target(root, relative)
    original_bytes = path.read_bytes() if path.exists() else None
    original = json.loads(original_bytes.decode('utf-8-sig'), object_pairs_hook=unique_launcher_properties) if original_bytes is not None else {}
    snapshot = hashlib.sha256(original_bytes).digest() if original_bytes is not None else None
    if not isinstance(original, dict):
        raise ValueError('LauncherSettings.json must contain an object')
    updated = copy.deepcopy(original)
    servers = updated.setdefault('Servers', [])
    if not isinstance(servers, list) or any(not isinstance(item, dict) for item in servers):
        raise ValueError('Launcher Servers must be an array of server objects')
    for server in servers:
        for field in ('Name', 'IpAddress', 'ServerId'):
            if field in server and not isinstance(server[field], str):
                raise ValueError('Launcher server fields must be strings: ' + field)
    address = server_address(CONFIG['server_url'])
    matches = [item for item in servers if isinstance(item.get('IpAddress'), str)
               and server_address(item['IpAddress']) == address]
    if len(matches) > 1:
        raise ValueError('Multiple launcher entries match this endpoint; resolve the ambiguity before setup')
    if matches:
        entry = matches[0]
        entry['IpAddress'] = address
        if not entry.get('Name'):
            entry['Name'] = CONFIG['server_name']
    else:
        entry = {'Name': CONFIG['server_name'], 'IpAddress': address}
        servers.append(entry)
    if not entry.get('ServerId'):
        # Current launcher's ServerAddDialog uses Unix seconds, not a profile ID.
        identifier = int(time.time())
        used = {str(item.get('ServerId')) for item in servers}
        while str(identifier) in used or identifier == 1721162719:
            identifier += 1
        entry['ServerId'] = str(identifier)
    if updated == original:
        return relative, None, snapshot
    return relative, (json.dumps(updated, ensure_ascii=False, indent=2) + '\n').encode('utf-8'), snapshot


def archive_name(info):
    name = info.filename.replace('\\', '/')
    components = name.rstrip('/').split('/')
    if not name or name.startswith('/') or any(not p or p in ('.', '..') for p in components):
        raise ValueError('Unsafe ZIP path: ' + info.filename)
    for part in components:
        if '~' in part or ':' in part or any(ord(c) < 32 for c in part) or part.endswith(('.', ' ')) or RESERVED.match(part):
            raise ValueError('Unsafe/non-portable ZIP path: ' + info.filename)
    first = components[0].lower()
    if first in ('spt', 'spt_runtime', 'user') or first.startswith('.quma-'):
        raise ValueError('Server/runtime/bootstrap data is not permitted in a client ZIP: ' + info.filename)
    if len(components) > 1 and first not in ('bepinex', 'escapefromtarkov_data', 'source'):
        raise ValueError('Unexpected client ZIP directory: ' + info.filename)
    if first in ('escapefromtarkov.exe', 'escapefromtarkov_be.exe'):
        raise ValueError('Client mod ZIP must not replace the owned game executable')
    kind = stat.S_IFMT(info.external_attr >> 16)
    if kind not in (0, stat.S_IFREG, stat.S_IFDIR) or info.flag_bits & 1:
        raise ValueError('Links, special files and encrypted entries are not permitted')
    return '/'.join(components)


def editable_config(name):
    return name.lower().startswith('bepinex/config/') and Path(name).suffix.lower() in CONFIG_SUFFIXES


def inspect_archive(archive, root):
    entries, seen, files = [], {}, set()
    for info in archive.infolist():
        name = archive_name(info)
        key = name.casefold()
        directory = info.is_dir() or stat.S_ISDIR(info.external_attr >> 16)
        if key in seen:
            raise ValueError('Duplicate/case-colliding ZIP path: ' + name)
        seen[key] = directory
        if not directory:
            files.add(key)
        safe_target(root, name, directory=directory)
        entries.append((info, name, directory))
    if not entries or len(entries) > 100000:
        raise ValueError('Client ZIP is empty or has too many entries')
    for key in seen:
        if any('/'.join(key.split('/')[:i]) in files for i in range(1, len(key.split('/')))):
            raise ValueError('ZIP file/directory collision: ' + key)
    by_name = {name: info for info, name, directory in entries if not directory}
    for required in CONFIG['required_files']:
        if required not in by_name or by_name[required].file_size <= 0:
            raise ValueError('Instance ZIP is missing required client component: ' + required)
    total = sum(info.file_size for info, _, directory in entries if not directory)
    backups = sum((root / name).stat().st_size for _, name, directory in entries
                  if not directory and (root / name).exists() and not editable_config(name))
    runtime = prepare_client(root)
    settings_relative, settings_bytes, _ = launcher_update(root, runtime)
    settings_path = safe_target(root, settings_relative)
    launcher_old = settings_path.stat().st_size if settings_path.exists() else 0
    launcher_new = len(settings_bytes) if settings_bytes is not None else launcher_old
    # Remaining space after download: staging + install/atomic new copies,
    # old backups + rollback atomic copies, and launcher copies in both phases.
    required_space = 2 * total + 2 * backups + 2 * (launcher_new + launcher_old) + MARGIN
    if shutil.disk_usage(root).free < required_space:
        raise ValueError('Insufficient free space for staging, backups and safety margin')
    return entries


def download(destination, root):
    parsed = urlsplit(CONFIG['archive_url'])
    if parsed.scheme != 'https' and not (parsed.scheme == 'http' and parsed.hostname in ('127.0.0.1', '::1', 'localhost')):
        raise ValueError('Archive download requires HTTPS')
    print('Downloading instance client pack (this can be several gigabytes)...', flush=True)
    request = Request(CONFIG['archive_url'], headers={'User-Agent': 'Quartermaster-Bootstrap'})
    with urlopen(request, timeout=60) as response:
        final = urlsplit(response.url)
        if parsed.scheme == 'https' and final.scheme != 'https':
            raise ValueError('Refusing HTTPS downgrade')
        length = response.headers.get('Content-Length')
        if length and shutil.disk_usage(root).free < int(length) + MARGIN:
            raise ValueError('Insufficient free space for archive download')
        with destination.open('xb') as output:
            shutil.copyfileobj(response, output, 1024 * 1024)
        if length and destination.stat().st_size != int(length):
            raise ValueError('Incomplete archive download')


def stage_archive(archive, entries, stage):
    staged = []
    for info, name, directory in entries:
        if directory:
            continue
        path = stage / name
        path.parent.mkdir(parents=True, exist_ok=True)
        # Reading to EOF verifies ZIP CRC before any client file is replaced.
        with archive.open(info) as source, path.open('xb') as destination:
            shutil.copyfileobj(source, destination, 1024 * 1024)
        if path.stat().st_size != info.file_size:
            raise ValueError('Incomplete ZIP entry: ' + name)
        staged.append((name, path))
    return staged


def digest(path):
    with path.open('rb') as stream:
        value = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(chunk)
        return value.digest()


def atomic_copy(source, destination):
    descriptor, name = tempfile.mkstemp(prefix='.quma-write-', dir=destination.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, 'wb') as output, source.open('rb') as incoming:
            shutil.copyfileobj(incoming, output, 1024 * 1024)
            output.flush()
            os.fsync(output.fileno())
        if destination.exists():
            os.chmod(temporary, stat.S_IMODE(destination.stat().st_mode))
        else:
            os.chmod(temporary, 0o644)
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def check_pending(root):
    backups = safe_target(root, '.quma-bootstrap-backups', directory=True)
    if backups.exists():
        for path in backups.iterdir():
            if path.is_symlink() or not path.is_dir():
                raise ValueError('Unexpected bootstrap backup entry: ' + str(path))
            manifest = safe_target(root, str(path.relative_to(root) / 'manifest.json'))
            if manifest.exists() and read_json(manifest).get('status') == 'applying':
                raise ValueError('An interrupted bootstrap needs recovery; retain backups at ' + str(path))


def verify_installed(root, settings_relative):
    for required in CONFIG['required_files']:
        path = safe_target(root, required)
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError('Installed client component failed verification: ' + required)
    saved = read_json(safe_target(root, settings_relative))
    if not any(item.get('IpAddress') == server_address(CONFIG['server_url']) for item in saved.get('Servers', [])):
        raise ValueError('Launcher server failed readback verification')


def apply_files(root, staged, settings_relative, settings_bytes, snapshot, temp):
    changes = []
    preserved = []
    for name, incoming in staged:
        destination = safe_target(root, name)
        if destination.exists() and editable_config(name):
            preserved.append(name)
            continue
        if destination.exists() and digest(destination) == digest(incoming):
            continue
        changes.append((name, incoming))
    if settings_bytes is not None:
        incoming = temp / 'LauncherSettings.json'
        incoming.write_bytes(settings_bytes)
        changes.append((settings_relative, incoming))
    if not changes:
        verify_installed(root, settings_relative)
        print('Client files and launcher are already configured.')
        return None, preserved
    backup_parent = safe_target(root, '.quma-bootstrap-backups', directory=True)
    backup_parent.mkdir(mode=0o700, exist_ok=True)
    backup = Path(tempfile.mkdtemp(prefix=time.strftime('%Y%m%dT%H%M%S-'), dir=backup_parent))
    manifest = {'status': 'applying', 'files': []}
    journal = backup / 'manifest.json'
    try:
        for name, incoming in changes:
            destination = safe_target(root, name)
            if name == settings_relative and (digest(destination) if destination.exists() else None) != snapshot:
                raise ValueError('Launcher settings changed during setup; close the launcher and retry')
            previous = backup / 'files' / name
            existed = destination.exists()
            if existed:
                previous.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(destination, previous)
            manifest['files'].append({'path': name, 'existed': existed})
            journal.write_text(json.dumps(manifest, indent=2), encoding='utf-8')
            destination.parent.mkdir(parents=True, exist_ok=True)
            safe_target(root, name)
            atomic_copy(incoming, destination)
        for name, incoming in changes:
            if digest(safe_target(root, name)) != digest(incoming):
                raise ValueError('Installed file failed verification: ' + name)
        verify_installed(root, settings_relative)
        manifest['status'] = 'complete'
        journal.write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    except BaseException:
        print('Setup failed; restoring replaced client files. Backups: ' + str(backup), file=sys.stderr)
        for change in reversed(manifest['files']):
            destination = safe_target(root, change['path'])
            if change['existed']:
                atomic_copy(backup / 'files' / change['path'], destination)
            else:
                destination.unlink(missing_ok=True)
        manifest['status'] = 'rolled_back'
        journal.write_text(json.dumps(manifest, indent=2), encoding='utf-8')
        raise
    return backup, preserved


def main():
    parser = argparse.ArgumentParser(description='Configure an existing matching SPT client for ' + CONFIG['server_name'])
    parser.add_argument('--spt-dir', type=Path, default=Path.cwd())
    parser.add_argument('--archive', type=Path, help='Use an already downloaded instance client ZIP')
    parser.add_argument('--yes', action='store_true', help='Apply an already-approved plan without prompting')
    parser.add_argument('--dry-run', '--check', action='store_true', help='Validate and print a plan without changing files')
    args = parser.parse_args()
    root = args.spt_dir.resolve(strict=True)
    runtime = prepare_client(root)
    check_pending(root)
    settings_relative, settings_bytes, snapshot = launcher_update(root, runtime)
    if args.archive:
        archive_path = args.archive.resolve(strict=True)
        with zipfile.ZipFile(archive_path) as archive:
            inspect_archive(archive, root)
    print('=== Quartermaster Bootstrap ===')
    print('Instance: ' + CONFIG['server_name'] + ' (' + CONFIG['server_url'] + ')')
    print('Target: ' + str(root))
    print('Required SPT: ' + CONFIG['spt_version'] + '; EFT compatibility: ' + CONFIG['tarkov_version'])
    print('Plan: install instance client mods; preserve existing editable configs; back up replaced files; merge launcher server.')
    if args.dry_run:
        print('Dry run: no files changed. Wine/Proton and game installation are not configured by this script.')
        return
    if not args.yes:
        print('Apply this plan? [y/N] ', end='', flush=True)
        # Bash source arrives on stdin; FD3 retains the caller\'s original input.
        try:
            answer = os.fdopen(os.dup(3)).readline() if os.name != 'nt' else input()
        except OSError:
            answer = input()
        if answer.strip().lower() not in ('y', 'yes'):
            raise ValueError('Setup cancelled; no client files changed')
    with tempfile.TemporaryDirectory(prefix='.quma-bootstrap-stage-', dir=root) as temporary:
        temp = Path(temporary)
        archive_path = args.archive.resolve(strict=True) if args.archive else temp / 'client-pack.zip'
        if not args.archive:
            download(archive_path, root)
        with zipfile.ZipFile(archive_path) as archive:
            entries = inspect_archive(archive, root)
            staged = stage_archive(archive, entries, temp / 'files')
        settings_path = safe_target(root, settings_relative)
        current_snapshot = digest(settings_path) if settings_path.exists() else None
        if current_snapshot != snapshot or prepare_client(root) != runtime:
            raise ValueError('Client/launcher settings changed during setup; close the launcher and retry')
        backup, preserved = apply_files(root, staged, settings_relative, settings_bytes, snapshot, temp)

        print('Preserved editable configurations: ' + str(len(preserved)))
        if backup:
            print('Backups: ' + str(backup))
    print('=== Setup Complete ===')
    print('Launch the existing SPT launcher and select ' + CONFIG['server_name'] + '.')
    print('Existing Wine/Proton configuration is unchanged; actual game startup is not tested by bootstrap.')


if __name__ == '__main__':
    if hasattr(signal, 'SIGTERM'):
        signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        main()
    except (Exception, KeyboardInterrupt) as error:
        print('ERROR: ' + str(error or 'Setup interrupted'), file=sys.stderr)
        sys.exit(1)
