#Requires -Version 5.1
<#
Configure an existing matching SPT client; no game or system-runtime install.
Example: .\quma-bootstrap.ps1 -SptDir 'C:\Games\SPT' -DryRun
Then rerun without -DryRun and confirm, or use -Yes for an approved plan.
#>
[CmdletBinding()]
param(
    [string]$SptDir = (Get-Location).Path,
    [string]$ArchivePath,
    [switch]$DryRun,
    [switch]$Yes
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$Config = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__QUMA_CONFIG_BASE64__')) | ConvertFrom-Json
$Utf8 = New-Object Text.UTF8Encoding($false)
$Margin = [long]16777216
$TempDirectory = $null
$Zip = $null

function Get-SafeTarget {
    param([string]$Root, [string]$Relative, [switch]$Directory)
    $parts = $Relative.Replace('\', '/').Split('/')
    $current = $Root
    for ($i = 0; $i -lt $parts.Length; $i++) {
        if ($parts[$i] -eq '' -or $parts[$i] -eq '.' -or $parts[$i] -eq '..' -or $parts[$i].Contains('~')) { throw 'Unsafe destination path' }
        $current = [IO.Path]::Combine($current, $parts[$i])
        try { $attributes = [IO.File]::GetAttributes($current) }
        catch [IO.FileNotFoundException] { continue }
        catch [IO.DirectoryNotFoundException] { continue }
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Destination contains a link/reparse point: $current" }
        if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
            $longLeaf = [IO.Path]::GetFileName([QumaBootstrap.Streams]::LongPath($current))
            if (-not [StringComparer]::OrdinalIgnoreCase.Equals($longLeaf, $parts[$i])) { throw "Destination uses a Windows short alias: $current" }
        }
        $isDirectory = ($attributes -band [IO.FileAttributes]::Directory) -ne 0
        if (($Directory -or $i -lt $parts.Length - 1) -and -not $isDirectory) { throw "Destination ancestor is not a directory: $current" }
        if (-not $Directory -and $i -eq $parts.Length - 1 -and $isDirectory) { throw "Destination is not a regular file: $current" }
    }
    return $current
}

function Read-JsonFile {
    param([string]$Path)
    return ([IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) | ConvertFrom-Json)
}

function Set-Property {
    param($Object, [string]$Name, $Value)
    $Object | Add-Member -MemberType NoteProperty -Name $Name -Value $Value -Force
}

function Get-ServerAddress {
    param([string]$Address)
    if ($Address -notmatch '://') { $Address = 'https://' + $Address }
    $uri = New-Object Uri($Address)
    if ($uri.Scheme -notin @('https', 'http') -or -not $uri.Host -or $uri.UserInfo -or $uri.Query -or $uri.Fragment -or $uri.AbsolutePath -ne '/') {
        throw 'Server address must contain only a host and optional port'
    }
    return $uri.Authority.ToLowerInvariant()
}

function Get-ClientRuntime {
    param([string]$Root)
    foreach ($relative in @('EscapeFromTarkov.exe', 'BepInEx')) {
        $path = Get-SafeTarget $Root $relative -Directory:($relative -eq 'BepInEx')
        if (-not (Test-Path -LiteralPath $path)) { throw "Existing owned SPT client required; missing $relative" }
    }
    $runtimes = @(@('SPT_Runtime', 'SPT') | Where-Object { Test-Path -LiteralPath ([IO.Path]::Combine($Root, "$_/SPT_Data/configs/core.json")) })
    if ($runtimes.Count -ne 1) { throw 'Expected exactly one unambiguous SPT runtime' }
    $runtime = $runtimes[0]
    $deps = Read-JsonFile (Get-SafeTarget $Root "$runtime/SPT.Server.deps.json")
    $versions = @($deps.libraries.PSObject.Properties.Name | Where-Object { $_.StartsWith('SPT.Server/') } | ForEach-Object { $_.Split('/')[1].Split('-')[0] })
    if ($versions.Count -ne 1 -or $versions[0] -cne $Config.spt_version) { throw "Client SPT version does not match this instance ($($Config.spt_version))" }
    $core = Read-JsonFile (Get-SafeTarget $Root "$runtime/SPT_Data/configs/core.json")
    $version = $core.PSObject.Properties['compatibleTarkovVersion']
    if (-not $version) { $version = $core.PSObject.Properties['compatible_tarkov_version'] }
    if (-not $version -or $version.Value -cne $Config.tarkov_version) { throw "Client EFT compatibility metadata does not match $($Config.tarkov_version)" }
    $launchers = @("$runtime/SPT.Launcher.exe", "$runtime/SPT.Launcher.Linux")
    $found = $false
    foreach ($relative in $launchers) { if ([IO.File]::Exists((Get-SafeTarget $Root $relative))) { $found = $true } }
    if (-not $found) { throw 'Current SPT launcher not found' }
    return $runtime
}

function Get-BytesHash {
    param([byte[]]$Bytes)
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($Bytes)).Replace('-', '') }
    finally { $hash.Dispose() }
}

function Get-LauncherUpdate {
    param([string]$Root, [string]$Runtime)
    $relative = "$Runtime/user/Launcher/LauncherSettings.json"
    $path = Get-SafeTarget $Root $relative
    $exists = [IO.File]::Exists($path)
    $snapshot = $null
    if ($exists) {
        $original = [IO.File]::ReadAllBytes($path)
        $json = [Text.Encoding]::UTF8.GetString($original).TrimStart([char]0xfeff)
        [QumaBootstrap.Streams]::ValidateJsonProperties($json)
        $settings = $json | ConvertFrom-Json
        $snapshot = Get-BytesHash $original
    } else { $settings = [PSCustomObject]@{} }
    if ($null -eq $settings -or $settings -isnot [PSCustomObject]) { throw 'LauncherSettings.json must contain an object' }
    $changed = -not $exists
    $property = $settings.PSObject.Properties['Servers']
    if ($null -eq $property) { Set-Property $settings 'Servers' @(); $property = $settings.PSObject.Properties['Servers']; $changed = $true }
    if ($null -eq $property.Value -or $property.Value -isnot [Array]) { throw 'Launcher Servers must be an array of server objects' }
    $servers = New-Object 'Collections.Generic.List[object]'
    $entry = $null
    $address = Get-ServerAddress $Config.server_url
    foreach ($server in $property.Value) {
        if ($null -eq $server -or $server -isnot [PSCustomObject]) { throw 'Launcher Servers must contain server objects' }
        foreach ($field in @('Name', 'IpAddress', 'ServerId')) {
            $value = $server.PSObject.Properties[$field]
            if ($value -and $value.Value -isnot [string]) { throw "Launcher server fields must be strings: $field" }
        }
        $servers.Add($server)
        $ip = $server.PSObject.Properties['IpAddress']
        if ($ip -and (Get-ServerAddress $ip.Value) -ceq $address) {
            if ($null -ne $entry) { throw 'Multiple launcher entries match this endpoint; resolve ambiguity before setup' }
            $entry = $server
        }
    }
    if ($null -eq $entry) {
        $entry = [PSCustomObject]@{ Name = $Config.server_name; IpAddress = $address }
        $servers.Add($entry)
        $changed = $true
    } elseif ($entry.IpAddress -cne $address) { Set-Property $entry 'IpAddress' $address; $changed = $true }
    $name = $entry.PSObject.Properties['Name']
    if (-not $name -or -not $name.Value) { Set-Property $entry 'Name' $Config.server_name; $changed = $true }
    $id = $entry.PSObject.Properties['ServerId']
    if (-not $id -or -not $id.Value) {
        # Current launcher ServerAddDialog uses Unix seconds, not a profile ID.
        $identifier = [long][Math]::Floor(([DateTime]::UtcNow - [DateTime]'1970-01-01').TotalSeconds)
        $used = @($servers | ForEach-Object { $p = $_.PSObject.Properties['ServerId']; if ($p) { [string]$p.Value } })
        while ([string]$identifier -in $used -or $identifier -eq 1721162719) { $identifier++ }
        Set-Property $entry 'ServerId' ([string]$identifier)
        $changed = $true
    }
    Set-Property $settings 'Servers' ($servers.ToArray())
    $bytes = $null
    if ($changed) { $bytes = $Utf8.GetBytes(($settings | ConvertTo-Json -Depth 100) + "`n") }
    return [PSCustomObject]@{ Relative = $relative; Bytes = $bytes; Snapshot = $snapshot }
}

function Test-EditableConfig {
    param([string]$Name)
    return $Name.ToLowerInvariant().StartsWith('bepinex/config/') -and [IO.Path]::GetExtension($Name).ToLowerInvariant() -in @('.cfg', '.ini', '.json', '.jsonc', '.yaml', '.yml', '.toml', '.xml')
}

function Get-FreeSpace {
    param([string]$Root)
    $drive = New-Object IO.DriveInfo([IO.Path]::GetPathRoot($Root))
    return $drive.AvailableFreeSpace
}

function Get-ArchiveEntries {
    param($Archive, [string]$Root)
    $seen = New-Object 'Collections.Generic.Dictionary[string,bool]' ([StringComparer]::OrdinalIgnoreCase)
    $files = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $entries = New-Object 'Collections.Generic.List[object]'
    [long]$total = 0
    [long]$backups = 0
    foreach ($entry in $Archive.Entries) {
        $name = $entry.FullName.Replace('\', '/')
        $parts = $name.TrimEnd('/').Split('/')
        if (-not $name -or $name.StartsWith('/')) { throw 'Unsafe ZIP path' }
        foreach ($part in $parts) {
            if (-not $part -or $part.Contains('~') -or $part -in @('.', '..') -or $part -match '[:\x00-\x1f]' -or $part.EndsWith('.') -or $part.EndsWith(' ') -or $part -match '^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|$)') { throw "Unsafe/non-portable ZIP path: $name" }
        }
        $first = $parts[0].ToLowerInvariant()
        if ($first -in @('spt', 'spt_runtime', 'user') -or $first.StartsWith('.quma-')) { throw 'Server/runtime/bootstrap data is not permitted in a client ZIP' }
        if ($parts.Length -gt 1 -and $first -notin @('bepinex', 'escapefromtarkov_data', 'source')) { throw "Unexpected client ZIP directory: $name" }
        if ($first -in @('escapefromtarkov.exe', 'escapefromtarkov_be.exe')) { throw 'Client ZIP must not replace the owned game executable' }
        $kind = ($entry.ExternalAttributes -shr 16) -band 61440
        if ($kind -notin @(0, 16384, 32768)) { throw 'Links and special ZIP entries are not permitted' }
        $directory = $name.EndsWith('/') -or $kind -eq 16384
        $name = $parts -join '/'
        if ($seen.ContainsKey($name)) { throw "Duplicate/case-colliding ZIP path: $name" }
        $seen.Add($name, $directory)
        if (-not $directory) { [void]$files.Add($name) }
        $target = Get-SafeTarget $Root $name -Directory:$directory
        $entries.Add([PSCustomObject]@{ Entry = $entry; Name = $name; Directory = $directory })
        if (-not $directory) {
            $total += $entry.Length
            if ([IO.File]::Exists($target) -and -not (Test-EditableConfig $name)) { $backups += (New-Object IO.FileInfo($target)).Length }
        }
    }
    if ($entries.Count -eq 0 -or $entries.Count -gt 100000) { throw 'Client ZIP is empty or has too many entries' }
    foreach ($name in $seen.Keys) {
        $parts = $name.Split('/')
        for ($i = 1; $i -lt $parts.Length; $i++) { if ($files.Contains(($parts[0..($i - 1)] -join '/'))) { throw 'ZIP file/directory collision' } }
    }
    foreach ($required in $Config.required_files) {
        $match = @($entries | Where-Object { $_.Name -ceq $required -and -not $_.Directory -and $_.Entry.Length -gt 0 })
        if ($match.Count -ne 1) { throw "Instance ZIP is missing required client component: $required" }
    }
    $launcher = Get-LauncherUpdate $Root (Get-ClientRuntime $Root)
    $settingsPath = Get-SafeTarget $Root $launcher.Relative
    [long]$launcherOld = 0
    if ([IO.File]::Exists($settingsPath)) { $launcherOld = (New-Object IO.FileInfo($settingsPath)).Length }
    [long]$launcherNew = $launcherOld
    if ($null -ne $launcher.Bytes) { $launcherNew = $launcher.Bytes.Length }
    # Remaining free space after download; reserve staged/installed new copies,
    # old backups/rollback atomic copies, and launcher copies in both phases.
    $requiredSpace = 2 * $total + 2 * $backups + 2 * ($launcherNew + $launcherOld) + $Margin
    if ((Get-FreeSpace $Root) -lt $requiredSpace) { throw 'Insufficient free space for staging, backups and safety margin' }
    return ,$entries.ToArray()
}

# ZipArchive in Windows PowerShell's .NET Framework does not check ZIP CRCs.
# This streaming helper checks decompressed bytes without per-byte PS overhead.
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using System.Runtime.InteropServices;
namespace QumaBootstrap {
 public static class Streams {
  // ConvertFrom-Json accepts non-JSON syntax and discards duplicate names.
  // Validate the complete strict grammar first, with one name set per object.
  public static void ValidateJsonProperties(string json) { new JsonProperties(json).Validate(); }
  sealed class JsonProperties {
   readonly string text;
   int position;
   public JsonProperties(string json) { text=json; }
   FormatException Invalid() { return new FormatException("Invalid launcher JSON syntax"); }
   char Peek { get { return position<text.Length ? text[position] : '\0'; } }
   void Space() { while(Peek==' ' || Peek=='\t' || Peek=='\r' || Peek=='\n') position++; }
   bool Take(char c) { if(position<text.Length && text[position]==c) { position++; return true; } return false; }
   void Need(char c) { if(!Take(c)) throw Invalid(); }
   bool Digit { get { return Peek>='0' && Peek<='9'; } }
   public void Validate() { Value(0); Space(); if(position!=text.Length) throw Invalid(); }
   void Value(int depth) {
    if(depth>1000) throw Invalid();
    Space();
    if(Take('{')) {
     var names=new System.Collections.Generic.HashSet<string>(StringComparer.OrdinalIgnoreCase);
     Space(); if(Take('}')) return;
     do {
      Space(); string name=String();
      if(!names.Add(name)) throw new FormatException("Duplicate launcher JSON property: " + name);
      Space(); Need(':'); Value(depth+1); Space();
      if(Take('}')) return;
      Need(',');
     } while(true);
    }
    if(Take('[')) {
     Space(); if(Take(']')) return;
     do { Value(depth+1); Space(); if(Take(']')) return; Need(','); } while(true);
    }
    if(Peek=='"') { String(); return; }
    if(Peek=='t') { Literal("true"); return; }
    if(Peek=='f') { Literal("false"); return; }
    if(Peek=='n') { Literal("null"); return; }
    Take('-');
    if(!Take('0')) { if(Peek<'1' || Peek>'9') throw Invalid(); while(Digit) position++; }
    if(Take('.')) { if(!Digit) throw Invalid(); while(Digit) position++; }
    if(Take('e') || Take('E')) {
     if(!Take('+')) Take('-');
     if(!Digit) throw Invalid(); while(Digit) position++;
    }
   }
   void Literal(string value) { foreach(char c in value) Need(c); }
   string String() {
    Need('"'); var value=new StringBuilder();
    while(position<text.Length) {
     char c=text[position++];
     if(c=='"') return value.ToString();
     if(c<32) throw Invalid();
     if(c!='\\') { value.Append(c); continue; }
     if(position==text.Length) throw Invalid();
     c=text[position++];
     switch(c) {
      case '"': case '\\': case '/': value.Append(c); break;
      case 'b': value.Append('\b'); break;
      case 'f': value.Append('\f'); break;
      case 'n': value.Append('\n'); break;
      case 'r': value.Append('\r'); break;
      case 't': value.Append('\t'); break;
      case 'u':
       int code=0;
       for(int i=0;i<4;i++) {
        if(position==text.Length) throw Invalid();
        char h=text[position++];
        int digit=h>='0' && h<='9' ? h-'0' : h>='a' && h<='f' ? h-'a'+10 : h>='A' && h<='F' ? h-'A'+10 : -1;
        if(digit<0) throw Invalid(); code=code*16+digit;
       }
       value.Append((char)code); break;
      default: throw Invalid();
     }
    }
    throw Invalid();
   }
  }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
  static extern uint GetLongPathNameW(string path, StringBuilder buffer, uint length);
  public static string LongPath(string path) {
   StringBuilder buffer=new StringBuilder(32768);
   uint length=GetLongPathNameW(path,buffer,(uint)buffer.Capacity);
   if(length==0 || length>=buffer.Capacity) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
   return buffer.ToString();
  }
  static readonly uint[] Table = MakeTable();
  static uint[] MakeTable() {
   uint[] t = new uint[256];
   for (uint i=0;i<256;i++) { uint c=i; for(int j=0;j<8;j++) c=(c&1)!=0 ? 0xedb88320u^(c>>1) : c>>1; t[i]=c; }
   return t;
  }
  public static void ReplaceFile(string source, string destination) {
   if(File.Exists(destination)) File.Replace(source,destination,null);
   else File.Move(source,destination);
  }
  public static uint CopyAndCrc(Stream source, Stream destination) {
   byte[] buffer=new byte[1024*1024]; uint crc=0xffffffffu; int n;
   while((n=source.Read(buffer,0,buffer.Length))>0) {
    destination.Write(buffer,0,n);
    for(int i=0;i<n;i++) crc=Table[(crc^buffer[i])&255]^(crc>>8);
   }
   return ~crc;
  }
 }
}
'@

function Get-ZipCrcs {
    param([string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        $length = [int][Math]::Min([long]65557, $stream.Length)
        [void]$stream.Seek(-$length, [IO.SeekOrigin]::End)
        $tail = $reader.ReadBytes($length)
        $eocd = -1
        for ($i = $length - 22; $i -ge 0; $i--) {
            if ([BitConverter]::ToUInt32($tail, $i) -eq 101010256 -and $i + 22 + [BitConverter]::ToUInt16($tail, $i + 20) -eq $length) { $eocd = $i; break }
        }
        if ($eocd -lt 0) { throw 'ZIP end record not found' }
        [long]$count = [BitConverter]::ToUInt16($tail, $eocd + 10)
        [long]$offset = [BitConverter]::ToUInt32($tail, $eocd + 16)
        if ($count -eq 65535 -or $offset -eq 4294967295) {
            [void]$stream.Seek($stream.Length - $length + $eocd - 20, [IO.SeekOrigin]::Begin)
            if ($reader.ReadUInt32() -ne 117853008) { throw 'ZIP64 locator not found' }
            [void]$reader.ReadUInt32()
            $zip64 = $reader.ReadUInt64()
            [void]$stream.Seek([long]$zip64, [IO.SeekOrigin]::Begin)
            $record = $reader.ReadBytes(56)
            if ($record.Length -ne 56 -or [BitConverter]::ToUInt32($record, 0) -ne 101075792) { throw 'Invalid ZIP64 record' }
            $count = [long][BitConverter]::ToUInt64($record, 32)
            $offset = [long][BitConverter]::ToUInt64($record, 48)
        }
        if ($count -gt 100000 -or $offset -ge $stream.Length) { throw 'Invalid ZIP central directory' }
        [void]$stream.Seek($offset, [IO.SeekOrigin]::Begin)
        $values = New-Object 'Collections.Generic.List[uint32]'
        for ($i = 0; $i -lt $count; $i++) {
            $header = $reader.ReadBytes(46)
            if ($header.Length -ne 46 -or [BitConverter]::ToUInt32($header, 0) -ne 33639248) { throw 'Invalid ZIP central directory entry' }
            if (([BitConverter]::ToUInt16($header, 8) -band 1) -ne 0) { throw 'Encrypted ZIP entries are not permitted' }
            $values.Add([BitConverter]::ToUInt32($header, 16))
            $skip = [int][BitConverter]::ToUInt16($header, 28) + [int][BitConverter]::ToUInt16($header, 30) + [int][BitConverter]::ToUInt16($header, 32)
            [void]$stream.Seek($skip, [IO.SeekOrigin]::Current)
        }
        return ,$values.ToArray()
    } finally { $reader.Dispose(); $stream.Dispose() }
}

function Write-AtomicFile {
    param([string]$Source, [string]$Destination)
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($Destination), '.quma-write-' + [Guid]::NewGuid().ToString('N'))
    try {
        [IO.File]::Copy($Source, $temporary, $false)
        [QumaBootstrap.Streams]::ReplaceFile($temporary, $Destination)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}

function Write-Journal {
    param([string]$Path, $Manifest)
    [IO.File]::WriteAllText($Path, ($Manifest | ConvertTo-Json -Depth 100), $Utf8)
}

function Test-PendingBootstrap {
    param([string]$Root)
    $parent = Get-SafeTarget $Root '.quma-bootstrap-backups' -Directory
    if ([IO.Directory]::Exists($parent)) {
        foreach ($path in [IO.Directory]::GetFileSystemEntries($parent)) {
            $relative = '.quma-bootstrap-backups/' + [IO.Path]::GetFileName($path)
            [void](Get-SafeTarget $Root $relative -Directory)
            $manifest = Get-SafeTarget $Root ($relative + '/manifest.json')
            if ([IO.File]::Exists($manifest) -and (Read-JsonFile $manifest).status -eq 'applying') { throw "Interrupted bootstrap needs recovery; retain backups at $path" }
        }
    }
}

function Test-InstalledFiles {
    param([string]$Root, [string]$SettingsRelative)
    foreach ($required in $Config.required_files) {
        $path = Get-SafeTarget $Root $required
        if (-not [IO.File]::Exists($path) -or (New-Object IO.FileInfo($path)).Length -eq 0) { throw "Installed component failed verification: $required" }
    }
    $saved = Read-JsonFile (Get-SafeTarget $Root $SettingsRelative)
    $matches = @($saved.Servers | Where-Object { $_.IpAddress -ceq (Get-ServerAddress $Config.server_url) })
    if ($matches.Count -eq 0) { throw 'Launcher server failed readback verification' }
}

function Install-StagedFiles {
    param([string]$Root, $Staged, $Launcher, [string]$Temporary)
    $changes = New-Object 'Collections.Generic.List[object]'
    $preserved = 0
    foreach ($file in $Staged) {
        $target = Get-SafeTarget $Root $file.Name
        if ([IO.File]::Exists($target) -and (Test-EditableConfig $file.Name)) { $preserved++; continue }
        if ([IO.File]::Exists($target) -and (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ceq (Get-FileHash -LiteralPath $file.Path -Algorithm SHA256).Hash) { continue }
        $changes.Add($file)
    }
    if ($null -ne $Launcher.Bytes) {
        $path = [IO.Path]::Combine($Temporary, 'LauncherSettings.json')
        [IO.File]::WriteAllBytes($path, $Launcher.Bytes)
        $changes.Add([PSCustomObject]@{ Name = $Launcher.Relative; Path = $path })
    }
    if ($changes.Count -eq 0) { Test-InstalledFiles $Root $Launcher.Relative; Write-Host 'Client files and launcher are already configured.'; return $preserved }
    $parent = Get-SafeTarget $Root '.quma-bootstrap-backups' -Directory
    [void][IO.Directory]::CreateDirectory($parent)
    $backup = [IO.Path]::Combine($parent, [DateTime]::UtcNow.ToString('yyyyMMddTHHmmss-') + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($backup)
    $journal = [IO.Path]::Combine($backup, 'manifest.json')
    $applied = New-Object 'Collections.Generic.List[object]'
    $manifest = [PSCustomObject]@{ status = 'applying'; files = @() }
    try {
        foreach ($file in $changes) {
            $target = Get-SafeTarget $Root $file.Name
            if ($file.Name -ceq $Launcher.Relative) {
                $current = $null
                if ([IO.File]::Exists($target)) { $current = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash }
                if ($current -cne $Launcher.Snapshot) { throw 'Launcher settings changed during setup; close the launcher and retry' }
            }
            $previous = [IO.Path]::Combine($backup, 'files/' + $file.Name)
            $existed = [IO.File]::Exists($target)
            if ($existed) { [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($previous)); [IO.File]::Copy($target, $previous, $false) }
            $applied.Add([PSCustomObject]@{ path = $file.Name; existed = $existed })
            $manifest.files = $applied.ToArray()
            Write-Journal $journal $manifest
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
            [void](Get-SafeTarget $Root $file.Name)
            Write-AtomicFile $file.Path $target
        }
        foreach ($file in $changes) {
            $target = Get-SafeTarget $Root $file.Name
            if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -cne (Get-FileHash -LiteralPath $file.Path -Algorithm SHA256).Hash) { throw 'Installed file failed verification' }
        }
        Test-InstalledFiles $Root $Launcher.Relative
        $manifest.status = 'complete'
        Write-Journal $journal $manifest
    } catch {
        Write-Host "Setup failed; restoring replaced files. Backups: $backup"
        for ($i = $applied.Count - 1; $i -ge 0; $i--) {
            $file = $applied[$i]
            $target = Get-SafeTarget $Root $file.path
            if ($file.existed) { Write-AtomicFile ([IO.Path]::Combine($backup, 'files/' + $file.path)) $target }
            elseif ([IO.File]::Exists($target)) { [IO.File]::Delete($target) }
        }
        $manifest.status = 'rolled_back'
        Write-Journal $journal $manifest
        throw
    }
    Write-Host "Backups: $backup"
    return $preserved
}

try {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Root = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $SptDir).ProviderPath)
    $Runtime = Get-ClientRuntime $Root
    Test-PendingBootstrap $Root
    $Launcher = Get-LauncherUpdate $Root $Runtime
    if ($ArchivePath) {
        $ArchivePath = (Resolve-Path -LiteralPath $ArchivePath).ProviderPath
        $Zip = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
        [void](Get-ArchiveEntries $Zip $Root)
        $Zip.Dispose(); $Zip = $null
    }
    Write-Host '=== Quartermaster Bootstrap ==='
    Write-Host "Instance: $($Config.server_name) ($($Config.server_url))"
    Write-Host "Target: $Root"
    Write-Host "Required SPT: $($Config.spt_version); EFT compatibility: $($Config.tarkov_version)"
    Write-Host 'Plan: install instance mods; preserve editable configs; back up replaced files; merge launcher server.'
    if ($DryRun) { Write-Host 'Dry run: no files changed. Game installation and system runtimes are not configured.'; exit 0 }
    if (-not $Yes) {
        Write-Host 'Apply this plan? [y/N] ' -NoNewline
        $answer = [Console]::ReadLine()
        if ($answer -notin @('y', 'yes')) { throw 'Setup cancelled; no client files changed' }
    }
    $TempDirectory = [IO.Path]::Combine($Root, '.quma-bootstrap-stage-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($TempDirectory)
    if (-not $ArchivePath) {
        $ArchivePath = [IO.Path]::Combine($TempDirectory, 'client-pack.zip')
        $uri = New-Object Uri($Config.archive_url)
        if ($uri.Scheme -ne 'https' -and -not ($uri.Scheme -eq 'http' -and $uri.IsLoopback)) { throw 'Archive download requires HTTPS' }
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Write-Host 'Downloading instance client pack (this can be several gigabytes)...'
        $request = [Net.WebRequest]::Create($uri)
        $request.Timeout = 60000; $request.ReadWriteTimeout = 60000
        $response = $request.GetResponse()
        try {
            if ($uri.Scheme -eq 'https' -and $response.ResponseUri.Scheme -ne 'https') { throw 'Refusing HTTPS downgrade' }
            if ($response.ContentLength -gt 0 -and (Get-FreeSpace $Root) -lt $response.ContentLength + $Margin) { throw 'Insufficient space for archive download' }
            $source = $response.GetResponseStream()
            $output = [IO.File]::Open($ArchivePath, [IO.FileMode]::CreateNew)
            try { $source.CopyTo($output, 1048576) } finally { $output.Dispose(); $source.Dispose() }
            if ($response.ContentLength -ge 0 -and (New-Object IO.FileInfo($ArchivePath)).Length -ne $response.ContentLength) { throw 'Incomplete archive download' }
        } finally { $response.Dispose() }
    }
    $Crcs = Get-ZipCrcs $ArchivePath
    $Zip = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    $Entries = Get-ArchiveEntries $Zip $Root
    if ($Crcs.Length -ne $Entries.Length) { throw 'ZIP directory entry count mismatch' }
    $Staged = New-Object 'Collections.Generic.List[object]'
    for ($i = 0; $i -lt $Entries.Length; $i++) {
        $item = $Entries[$i]
        if ($item.Directory) { continue }
        $path = [IO.Path]::Combine($TempDirectory, 'files/' + $item.Name)
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
        $source = $item.Entry.Open()
        $output = [IO.File]::Open($path, [IO.FileMode]::CreateNew)
        try { $crc = [QumaBootstrap.Streams]::CopyAndCrc($source, $output) }
        finally { $output.Dispose(); $source.Dispose() }
        if ($crc -ne $Crcs[$i] -or (New-Object IO.FileInfo($path)).Length -ne $item.Entry.Length) { throw "ZIP checksum/length mismatch: $($item.Name)" }
        $Staged.Add([PSCustomObject]@{ Name = $item.Name; Path = $path })
    }
    $Zip.Dispose(); $Zip = $null
    $settingsPath = Get-SafeTarget $Root $Launcher.Relative
    $currentSnapshot = $null
    if ([IO.File]::Exists($settingsPath)) { $currentSnapshot = (Get-FileHash -LiteralPath $settingsPath -Algorithm SHA256).Hash }
    if ($currentSnapshot -cne $Launcher.Snapshot -or (Get-ClientRuntime $Root) -cne $Runtime) { throw 'Client/launcher settings changed during setup; close the launcher and retry' }
    $Preserved = Install-StagedFiles $Root $Staged $Launcher $TempDirectory

    Write-Host "Preserved editable configurations: $Preserved"
    Write-Host '=== Setup Complete ==='
    Write-Host "Launch the existing SPT launcher and select $($Config.server_name)."
    Write-Host 'System runtimes are unchanged; actual game startup is not tested by bootstrap.'
} catch {
    Write-Host ('ERROR: ' + $_.Exception.Message)
    exit 1
} finally {
    if ($null -ne $Zip) { $Zip.Dispose() }
    if ($null -ne $TempDirectory -and [IO.Directory]::Exists($TempDirectory)) { [IO.Directory]::Delete($TempDirectory, $true) }
}
