<#
.SYNOPSIS
    End-to-end test of the `pman` binary on Windows.

.DESCRIPTION
    Builds pman, runs the unit/integration tests, then drives the real binary
    against a throw-away sandbox: fake projects with back-dated mtimes, real git
    repositories, and its own ledger and strip profiles. Nothing outside the
    sandbox is touched unless -WithAutoRun is given.

    Works on Windows PowerShell 5.1 and PowerShell 7+. Requires git on PATH.

.PARAMETER Release
    Build and test the release binary.
.PARAMETER Bin
    Test an existing pman.exe instead of building one.
.PARAMETER SkipCargo
    Skip `cargo test`.
.PARAMETER WithAutoRun
    Also install/remove the real scheduled task. Skipped if a pManager task is
    already installed, so yours is never replaced.
.PARAMETER Keep
    Keep the sandbox even when everything passes.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\e2e.ps1
.EXAMPLE
    pwsh scripts/e2e.ps1 -Release -WithAutoRun
#>
[CmdletBinding()]
param(
    [switch]$Release,
    [string]$Bin,
    [switch]$SkipCargo,
    [switch]$WithAutoRun,
    [switch]$Keep
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$OnWindows = ($PSVersionTable.PSEdition -eq 'Desktop') -or $IsWindows
$Exe = if ($OnWindows) { 'pman.exe' } else { 'pman' }

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Error "git is required on PATH (winget install Git.Git)"
    exit 2
}

# Plain output for greppable logs; no inherited strip dir.
$env:NO_COLOR = '1'
Remove-Item Env:PMANAGER_STRIP_DIR, Env:RUST_LOG -ErrorAction SilentlyContinue

# ── reporting ────────────────────────────────────────────────────────────────
$script:Passed = 0
$script:Failed = 0
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Ctx = ''

function Section([string]$Name) { Write-Host "`n== $Name ==" -ForegroundColor Cyan }
function Ok([string]$Desc) { $script:Passed++; Write-Host "  PASS $Desc" -ForegroundColor Green }
function Bad([string]$Desc, [string]$Detail = '') {
    $script:Failed++; $script:Failures.Add($Desc)
    Write-Host "  FAIL $Desc" -ForegroundColor Red
    if ($Detail) { $Detail -split "`n" | ForEach-Object { Write-Host "       | $_" } }
}
function Skip([string]$Desc) { Write-Host "  SKIP $Desc" -ForegroundColor Yellow }

# Check DESC { condition }
function Check([string]$Desc, [scriptblock]$Cond) {
    $result = $false
    try { $result = [bool](& $Cond) } catch { $script:Ctx += "`nthrew: $_" }
    if ($result) { Ok $Desc } else { Bad $Desc $script:Ctx }
}

# ── build ────────────────────────────────────────────────────────────────────
$BuildProfile = if ($Release) { 'release' } else { 'debug' }
$CargoFlags = @(); if ($Release) { $CargoFlags = @('--release') }
$Manifest = Join-Path $Root 'Cargo.toml'

if (-not $Bin) {
    Section "build ($BuildProfile)"
    & cargo build --manifest-path $Manifest @CargoFlags
    if ($LASTEXITCODE -ne 0) { Bad 'cargo build'; exit 1 }
    Ok 'cargo build'
    $Bin = Join-Path $Root "target\$BuildProfile\$Exe"
}
$Bin = (Resolve-Path -LiteralPath $Bin).Path

if (-not $SkipCargo) {
    Section 'cargo test'
    & cargo test --manifest-path $Manifest @CargoFlags --quiet
    if ($LASTEXITCODE -eq 0) { Ok 'cargo test' } else { Bad 'cargo test' }
}

# ── sandbox ──────────────────────────────────────────────────────────────────
# The space in the name exercises argument quoting all the way through.
$W = Join-Path ([IO.Path]::GetTempPath()) ("pman e2e " + [guid]::NewGuid().ToString('N').Substring(0, 8))
$Proj = Join-Path $W 'projects'
$Strip = Join-Path $W 'strip'
$Db = Join-Path $W 'ledger\pmanager.db'
New-Item -ItemType Directory -Force -Path $Proj, $Strip | Out-Null

# Quote one argument by the rules CommandLineToArgvW / the Rust runtime use.
function Quote-Arg([string]$A) {
    if ($A -ne '' -and $A -notmatch '[\s"]') { return $A }
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    $bs = 0
    foreach ($c in $A.ToCharArray()) {
        if ($c -eq '\') { $bs++; continue }
        if ($c -eq '"') { [void]$sb.Append('\' * (2 * $bs + 1)).Append('"'); $bs = 0; continue }
        [void]$sb.Append('\' * $bs).Append($c); $bs = 0
    }
    [void]$sb.Append('\' * (2 * $bs)).Append('"')
    $sb.ToString()
}

# Pman ARGS...  -> sets $OUT (stdout), $ERR (stderr), $RC. stdin is never a
# console. A Process is used rather than `& pman 2>file`, because Windows
# PowerShell 5.1 wraps native stderr in ErrorRecords and breaks substring checks.
$script:OUT = ''; $script:ERR = ''; $script:RC = 0
function Pman {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Bin
    $psi.Arguments = ($args | ForEach-Object { Quote-Arg ([string]$_) }) -join ' '
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [Text.Encoding]::UTF8
    $proc = [Diagnostics.Process]::Start($psi)
    $proc.StandardInput.Close()
    $o = $proc.StandardOutput.ReadToEndAsync()
    $e = $proc.StandardError.ReadToEndAsync()
    $proc.WaitForExit()
    $script:OUT = $o.Result.Trim()
    $script:ERR = $e.Result.Trim()
    $script:RC = $proc.ExitCode
    $script:Ctx = "exit=$($script:RC)`nstdout: $($script:OUT)`nstderr: $($script:ERR)"
}
# P ARGS... -> pman against the main sandbox ledger and profiles
function P { Pman --db $Db --strip-dir $Strip @args }

function Succeeds { $script:RC -eq 0 }
function FailsWith([string]$Text) { $script:RC -ne 0 -and $script:ERR.Contains($Text) }

# JSON helpers. Piping enumerates, which sidesteps 5.1's unrolled-array quirk.
function Rows([string]$Json) {
    if (-not $Json) { return , @() }
    , @((ConvertFrom-Json -InputObject $Json) | ForEach-Object { $_ })
}
function Field($Rows, [string]$Name, [string]$Key) {
    $row = $Rows | Where-Object { $_.name -eq $Name } | Select-Object -First 1
    if ($null -eq $row) { return $null }
    $row.$Key
}
function Sum($Rows, [string]$Key) {
    $s = 0; foreach ($r in $Rows) { $s += [int64]$r.$Key }; $s
}

function Write-Text([string]$Path, [string]$Text) {
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    [IO.File]::WriteAllText($Path, $Text)   # UTF-8, no BOM
}
function Write-Zeros([string]$Path, [int]$Bytes) {
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    [IO.File]::WriteAllBytes($Path, (New-Object byte[] $Bytes))
}
function Touch([string]$Path) { (Get-Item -LiteralPath $Path -Force).LastWriteTimeUtc = [DateTime]::UtcNow }

# Set-Age DIR DAYS -> back-date every file and folder under DIR
function Set-Age([string]$Dir, [int]$Days) {
    $t = [DateTime]::UtcNow.AddDays(-$Days)
    Get-ChildItem -LiteralPath $Dir -Recurse -Force | ForEach-Object {
        try { $_.LastWriteTimeUtc = $t } catch { }   # .git internals are not activity anyway
    }
    (Get-Item -LiteralPath $Dir -Force).LastWriteTimeUtc = $t
}

# Local EAP: on 5.1, `2>&1` on a native command throws under 'Stop'.
function Git-Quiet {
    $ErrorActionPreference = 'Continue'
    & git @args 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "git $args failed" }
}
# Git-Repo DIR -> init with a local identity, commit everything
function Git-Repo([string]$Dir) {
    Git-Quiet -C $Dir -c init.defaultBranch=main init -q
    Git-Quiet -C $Dir config user.name pman-e2e
    Git-Quiet -C $Dir config user.email pman-e2e@example.invalid
    Git-Quiet -C $Dir config core.autocrlf false
    Git-Quiet -C $Dir add -A
    Git-Quiet -C $Dir commit -qm seed
}

function Python-Project([string]$Dir) {
    Write-Text (Join-Path $Dir 'pyproject.toml') "[project]`n"
    Write-Text (Join-Path $Dir 'main.py') "print('hi')`n"
    Write-Zeros (Join-Path $Dir '.venv\lib\big.bin') 20000
}
function Node-Project([string]$Dir) {
    Write-Text (Join-Path $Dir 'package.json') "{`"name`":`"x`"}`n"
    Write-Text (Join-Path $Dir 'index.js') "console.log(1)`n"
    Write-Zeros (Join-Path $Dir 'node_modules\left-pad\index.js') 20000
}

# Strip profiles in the format the README documents.
function Write-Profiles([string]$Dir, [string]$NodeLanguage = 'node') {
    Write-Text (Join-Path $Dir 'profiles.json') @'
[
  { "language": "python", "file": "./python.profile.json" },
  { "language": "node",   "file": "./node.profile.json" }
]
'@
    Write-Text (Join-Path $Dir 'python.profile.json') @'
{
  "language": "python",
  "detection":  [ { "type": "file", "element": "pyproject.toml" },
                  { "type": "folder", "element": ".venv" } ],
  "strippable": [ { "type": "folder", "element": ".venv" } ]
}
'@
    Write-Text (Join-Path $Dir 'node.profile.json') (@'
{
  "language": "LANG",
  "detection":  [ { "type": "file", "element": "package.json" } ],
  "strippable": [ { "type": "folder", "element": "node_modules" } ]
}
'@ -replace 'LANG', $NodeLanguage)
}
Write-Profiles $Strip

function Is-7z([string]$Path) {
    $fs = [IO.File]::OpenRead($Path)
    try { $b = New-Object byte[] 6; [void]$fs.Read($b, 0, 6) } finally { $fs.Dispose() }
    (($b | ForEach-Object { $_.ToString('x2') }) -join '') -eq '377abcaf271c'
}
function J([string]$Name) { Join-Path $Proj $Name }

try {
# ── CLI surface ──────────────────────────────────────────────────────────────
Section 'CLI surface'
Pman --version;  Check '--version prints the version' { $RC -eq 0 -and $OUT -like 'pman *' }
Pman --help;     Check '--help lists every command' { $OUT -like '*auto-run*' -and $OUT -like '*list*' -and $OUT -like '*project*' -and $OUT -like '*run*' }
Pman --help;     Check '--help shows `pman`, not `pman.exe`' { $OUT -notlike '*pman.exe*' }
Pman bogus;      Check 'an unknown command is rejected' { $RC -ne 0 }
Pman -v -q list; Check '-v and -q conflict' { $RC -ne 0 }
P list -o json;  Check 'list on a fresh ledger is an empty JSON array' { $RC -eq 0 -and (Rows $OUT).Count -eq 0 }
Check 'the ledger file is created on first use' { Test-Path -LiteralPath $Db }
P list;          Check 'list (table) on an empty ledger succeeds' { Succeeds }

# ── project add ──────────────────────────────────────────────────────────────
Section 'project add'

Python-Project (J fresh);       Set-Age (J fresh) 10
Python-Project (J strip-me);    Set-Age (J strip-me) 70
Write-Text (J 'no-profile\readme.txt') "hi`n"; Set-Age (J no-profile) 70
Node-Project (J node-old);      Set-Age (J node-old) 130

# A clean repo: the strippable folder is ignored, so the tree is not dirty.
Node-Project (J git-clean)
Write-Text (J 'git-clean\.gitignore') "node_modules/`n"
Git-Repo (J git-clean);         Set-Age (J git-clean) 130

# Dirty (untracked file): archiving is deferred to 160 days, stripping is not.
Node-Project (J git-dirty)
Write-Text (J 'git-dirty\.gitignore') "node_modules/`n"
Git-Repo (J git-dirty);         Write-Text (J 'git-dirty\notes.txt') "wip`n"
Set-Age (J git-dirty) 130

Write-Text (J 'git-dirty-old\readme.txt') "hi`n"
Git-Repo (J git-dirty-old);     Write-Text (J 'git-dirty-old\notes.txt') "wip`n"
Set-Age (J git-dirty-old) 170

# .pmanagerignore: a fresh file under an ignored folder is not activity.
Write-Text (J 'ignored\readme.txt') "hi`n"
Write-Text (J 'ignored\.pmanagerignore') "build/`n"
Write-Text (J 'ignored\build\out.bin') "out`n"
Set-Age (J ignored) 90;         Touch (J 'ignored\build\out.bin')
# Same tree without the ignore file: the fresh file makes it active.
Write-Text (J 'not-ignored\readme.txt') "hi`n"
Write-Text (J 'not-ignored\build\out.bin') "out`n"
Set-Age (J not-ignored) 90;     Touch (J 'not-ignored\build\out.bin')

Write-Text (J 'vanished\readme.txt') "hi`n"

$All = @('fresh', 'strip-me', 'no-profile', 'node-old', 'git-clean', 'git-dirty', 'git-dirty-old', 'ignored', 'not-ignored', 'vanished')
foreach ($n in $All) { P project add (J $n) --name $n; Check "add $n" { Succeeds } }

P list -o json; $List = Rows $OUT
Check "list shows all $($All.Count) projects" { $List.Count -eq $All.Count }
Check "every new project is 'registered'" { @($List | Where-Object { $_.state -ne 'registered' }).Count -eq 0 }
Check 'idle days come from the back-dated mtimes' { (Field $List 'strip-me' idle) -eq 70 }
$Paths = @{}; foreach ($n in $All) { $Paths[$n] = Field $List $n path }
Check 'stored paths are absolute, without a \\?\ prefix' {
    [IO.Path]::IsPathRooted($Paths['fresh']) -and $Paths['fresh'] -like '*\projects\fresh' -and $Paths['fresh'] -notlike '\\?\*'
}

P project add (Join-Path (J fresh) '..\fresh\.') --name other
Check 'the same folder spelled differently is a duplicate' { FailsWith 'already registered' }
P project add ((J fresh).ToUpperInvariant()) --name other
Check 'paths are case-insensitive on Windows' { FailsWith 'already registered' }
P project add ((J fresh).Replace('\', '/')) --name other
Check 'forward slashes name the same folder' { FailsWith 'already registered' }
New-Item -ItemType Directory -Force -Path (J spare) | Out-Null
P project add (J spare) --name fresh
Check 'a name already in use is refused' { FailsWith 'already used' }
P project add (J spare) --name '   '
Check 'a blank name is refused' { FailsWith 'must not be empty' }
P project add (J spare)
Check 'no --name without a console asks for --name' { FailsWith 'pass --name' }
P project add (J does-not-exist) --name ghost
Check 'a missing folder is refused' { FailsWith 'could not resolve' }
Write-Text (J a-file) "x"
P project add (J a-file) --name file
Check 'a file is not a project' { FailsWith 'is not a directory' }
P list -o json
Check 'failed adds left the ledger unchanged' { (Rows $OUT).Count -eq $All.Count }

Remove-Item -LiteralPath (J vanished) -Recurse -Force

# ── strip profiles ───────────────────────────────────────────────────────────
Section 'strip profile validation'
Pman --db $Db --strip-dir (Join-Path $W 'no-such-dir') run --dry-run
Check 'a missing strip directory fails the run' { $RC -ne 0 }
$StripBad = Join-Path $W 'strip-bad'
Write-Profiles $StripBad 'deno'
Pman --db $Db --strip-dir $StripBad run
Check 'a profile whose language disagrees with the index fails the run' { FailsWith 'invalid' }
Check '...and nothing was stripped' { Test-Path -LiteralPath (J 'strip-me\.venv') }

# ── run --dry-run ────────────────────────────────────────────────────────────
Section 'run --dry-run'
$WantAction = @{ 'fresh' = 'none'; 'strip-me' = 'strip'; 'no-profile' = 'none'; 'node-old' = 'strip+archive'
    'git-clean' = 'strip+archive'; 'git-dirty' = 'strip'; 'git-dirty-old' = 'archive'; 'ignored' = 'none'
    'not-ignored' = 'none'; 'vanished' = 'none' }
$WantResult = @{ 'fresh' = 'active'; 'strip-me' = 'dry-run'; 'no-profile' = 'no profile'; 'node-old' = 'dry-run'
    'git-clean' = 'dry-run'; 'git-dirty' = 'dry-run'; 'git-dirty-old' = 'dry-run'; 'ignored' = 'no profile'
    'not-ignored' = 'active'; 'vanished' = 'missing' }

P run --dry-run -o json; $Dry = Rows $OUT
Check 'dry run succeeds' { $RC -eq 0 }
foreach ($n in $All) {
    Check "dry run: $n -> $($WantAction[$n]) / $($WantResult[$n])" {
        (Field $Dry $n action) -eq $WantAction[$n] -and (Field $Dry $n result) -eq $WantResult[$n]
    }
}
Check 'dry run frees nothing' { (Sum $Dry freed) -eq 0 }
Check '.pmanagerignore keeps build/ out of the idle clock' { (Field $Dry ignored idle) -ge 89 }
Check 'without it, the fresh build file counts' { (Field $Dry not-ignored idle) -eq 0 }
Check 'dry run deleted nothing' {
    (Test-Path -LiteralPath (J 'strip-me\.venv')) -and (Test-Path -LiteralPath (J 'node-old\node_modules')) -and (Test-Path -LiteralPath (J git-dirty-old))
}
Check 'dry run wrote no archive' { @(Get-ChildItem -LiteralPath $Proj -Filter '*.pproject*').Count -eq 0 }
P list -o json
Check 'dry run changed no state' { @((Rows $OUT) | Where-Object { $_.state -ne 'registered' }).Count -eq 0 }

$env:PMANAGER_STRIP_DIR = $Strip
Pman --db $Db run --dry-run -o json
Remove-Item Env:PMANAGER_STRIP_DIR
Check 'PMANAGER_STRIP_DIR is honoured' { $RC -eq 0 -and (Rows $OUT).Count -eq $All.Count }

P run --dry-run
Check 'dry run renders as a table' { $RC -eq 0 -and $OUT -like '*NAME*' -and $OUT -like '*ACTION*' }

# ── run ──────────────────────────────────────────────────────────────────────
Section 'run'
P run -o json; $Run = Rows $OUT
Check 'run succeeds' { $RC -eq 0 }
foreach ($n in $All) {
    $want = if ($WantResult[$n] -eq 'dry-run') { 'ok' } else { $WantResult[$n] }
    Check "run: $n -> $($WantAction[$n]) / $want" {
        (Field $Run $n action) -eq $WantAction[$n] -and (Field $Run $n result) -eq $want
    }
}

Check 'strip: .venv removed' { -not (Test-Path -LiteralPath (J 'strip-me\.venv')) }
Check 'strip: sources kept' { (Test-Path -LiteralPath (J 'strip-me\main.py')) -and (Test-Path -LiteralPath (J 'strip-me\pyproject.toml')) }
Check 'strip: freed bytes reported' { (Field $Run strip-me freed) -ge 20000 }
Check 'untouched project kept its .venv' { Test-Path -LiteralPath (J 'fresh\.venv') }
Check 'no-profile project untouched' { Test-Path -LiteralPath (J 'no-profile\readme.txt') }
foreach ($n in 'node-old', 'git-clean', 'git-dirty-old') {
    # git-clean / git-dirty-old also prove read-only .git\objects can be removed.
    Check "archive: $n folder removed" { -not (Test-Path -LiteralPath (J $n)) }
    Check "archive: $n.pproject is a 7z file" { (Test-Path -LiteralPath (J "$n.pproject")) -and (Is-7z (J "$n.pproject")) }
    Check "archive: no .part left for $n" { -not (Test-Path -LiteralPath (J "$n.pproject.part")) }
}
Check 'dirty repo: stripped but not archived' { (Test-Path -LiteralPath (J git-dirty)) -and -not (Test-Path -LiteralPath (J 'git-dirty\node_modules')) }
Check 'dirty repo: uncommitted work kept' { Test-Path -LiteralPath (J 'git-dirty\notes.txt') }

P list -o json; $List = Rows $OUT
$WantState = @{ 'fresh' = 'registered'; 'strip-me' = 'stripped'; 'no-profile' = 'registered'; 'node-old' = 'archived'
    'git-clean' = 'archived'; 'git-dirty' = 'stripped'; 'git-dirty-old' = 'archived'; 'ignored' = 'registered'
    'not-ignored' = 'registered'; 'vanished' = 'registered' }
foreach ($n in $All) { Check "state: $n is $($WantState[$n])" { (Field $List $n state) -eq $WantState[$n] } }

Section 'second run'
P run -o json; $Run2 = Rows $OUT
Check 'second run succeeds' { Succeeds }
Check 'archived projects are left alone' { (Field $Run2 node-old result) -eq 'already archived' }
Check 'stripped project is not re-stripped' { (Field $Run2 strip-me action) -eq 'none' -and (Field $Run2 strip-me result) -eq 'active' }
Check 'a gone folder is reported missing' { (Field $Run2 vanished result) -eq 'missing' }
Check 'second run frees nothing' { (Sum $Run2 freed) -eq 0 }

# ── failure isolation ────────────────────────────────────────────────────────
Section 'one failing project does not stop the pass'
$Db2 = Join-Path $W 'ledger\failing.db'
$Iso = Join-Path $W 'iso'
Write-Text (Join-Path $Iso 'bad-ignore\readme.txt') "hi`n"
Node-Project (Join-Path $Iso 'sibling')
Pman --db $Db2 --strip-dir $Strip project add (Join-Path $Iso 'bad-ignore') --name bad-ignore
Pman --db $Db2 --strip-dir $Strip project add (Join-Path $Iso 'sibling') --name sibling
Write-Text (Join-Path $Iso 'bad-ignore\.pmanagerignore') "build/{`n"   # broken after registration
Set-Age $Iso 70
Pman --db $Db2 --strip-dir $Strip run -o json; $IsoRows = Rows $OUT
Check 'the run exits non-zero' { $RC -ne 0 }
Check 'the error names .pmanagerignore' { $ERR.Contains('.pmanagerignore') }
Check "the broken project's row says failed" { (Field $IsoRows bad-ignore result) -eq 'failed' }
Check 'its sibling was still stripped' {
    (Field $IsoRows sibling result) -eq 'ok' -and -not (Test-Path -LiteralPath (Join-Path $Iso 'sibling\node_modules'))
}

# A file held open (an editor, an indexer) fails that project only.
$Db3 = Join-Path $W 'ledger\locked.db'
$Locked = Join-Path $W 'locked\held'
Node-Project $Locked
Pman --db $Db3 --strip-dir $Strip project add $Locked --name held
Set-Age (Split-Path -Parent $Locked) 70
if ($OnWindows) {
    $handle = [IO.File]::Open((Join-Path $Locked 'node_modules\left-pad\index.js'), 'Open', 'Read', 'None')
    try { Pman --db $Db3 --strip-dir $Strip run -o json } finally { $handle.Dispose() }
    Check 'a locked file fails the strip instead of half-reporting it' { $RC -ne 0 -and (Field (Rows $OUT) held result) -eq 'failed' }
    Pman --db $Db3 --strip-dir $Strip list -o json
    Check '...and leaves the row registered for the next pass' { (Field (Rows $OUT) held state) -eq 'registered' }
    Pman --db $Db3 --strip-dir $Strip run -o json
    Check '...which succeeds once the file is released' { $RC -eq 0 -and (Field (Rows $OUT) held result) -eq 'ok' }
} else {
    Skip 'locked-file test (Windows only)'
}

# ── remove & cleanup ─────────────────────────────────────────────────────────
Section 'project remove'
P project remove $Paths['node-old']
Check 'remove an archived project' { Succeeds }
Check '...warns that its archive is left in place' { $ERR.Contains('left in place') }
Check '...and leaves the .pproject on disk' { Test-Path -LiteralPath (J 'node-old.pproject') }
P project remove $Paths['node-old']
Check 'removing it again fails' { FailsWith 'no project is registered' }
P project remove (Join-Path (J fresh) '..\fresh')
Check 'remove accepts any spelling of an existing folder' { Succeeds }
Check "...and never deletes the project's files" { Test-Path -LiteralPath (J 'fresh\main.py') }
P project remove (J never-registered)
Check 'removing an unknown path fails' { FailsWith 'no project is registered' }

Section 'project cleanup'
Remove-Item -LiteralPath (J no-profile) -Recurse -Force
P project cleanup -o json; $Clean = Rows $OUT
Check 'cleanup succeeds' { Succeeds }
Check 'cleanup removes a vanished registered project' { (Field $Clean vanished result) -eq 'removed' }
Check 'cleanup removes a deleted project' { (Field $Clean no-profile result) -eq 'removed' }
Check 'cleanup keeps archived rows' { (Field $Clean git-clean result) -eq 'kept' }
Check 'cleanup keeps existing projects' { (Field $Clean strip-me result) -eq 'kept' }
P list -o json
Check 'list reflects remove + cleanup' { (Rows $OUT).Count -eq ($All.Count - 4) }
P project cleanup -o json
Check 'a second cleanup removes nothing' { $RC -eq 0 -and @((Rows $OUT) | Where-Object { $_.result -eq 'removed' }).Count -eq 0 }

# ── auto-run ─────────────────────────────────────────────────────────────────
Section 'auto-run'
$Task = 'pManager-run'
function Task-Exists {
    $ErrorActionPreference = 'Continue'
    & schtasks /Query /TN $Task 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}
$HadTask = $OnWindows -and (Task-Exists)

Pman --db $Db --strip-dir (Join-Path $W 'no-such-dir') auto-run add
Check 'auto-run add refuses broken strip profiles' { $RC -ne 0 }
if ($OnWindows -and -not $HadTask) {
    Check '...before touching the scheduler' { -not (Task-Exists) }
}

if (-not $WithAutoRun) {
    Skip 'scheduler install/remove (pass -WithAutoRun)'
} elseif (-not $OnWindows) {
    Skip 'scheduled task test (Windows only; use e2e.zsh elsewhere)'
} elseif ($HadTask) {
    Skip "a '$Task' task is already installed; not replacing it"
} else {
    P auto-run add
    Check 'auto-run add installs the task' { $RC -eq 0 -and $ERR.Contains('installed') }
    Check "...visible to schtasks as $Task" { Task-Exists }
    $xml = (& schtasks /Query /TN $Task /XML) -join "`n"
    Check '...running pman with the sandbox ledger' { $xml.Contains('pmanager.db') -and $xml.Contains(' run') }
    P auto-run add
    Check 'adding it again changes nothing' { $RC -eq 0 -and $ERR.Contains('already installed') }
    P auto-run remove
    Check 'auto-run remove removes it' { $RC -eq 0 -and $ERR.Contains('removed') -and -not (Task-Exists) }
    P auto-run remove
    Check 'removing it again is a quiet no-op' { $RC -eq 0 -and $ERR.Contains('no scheduled run') }
}
}
catch {
    Bad "script error: $_" $_.ScriptStackTrace
}
finally {
    # ── summary ──────────────────────────────────────────────────────────────
    $color = if ($script:Failed -eq 0) { 'Green' } else { 'Red' }
    Write-Host "`n$($script:Passed) passed, $($script:Failed) failed" -ForegroundColor $color
    foreach ($f in $script:Failures) { Write-Host "  x $f" -ForegroundColor Red }

    if ($script:Failed -eq 0 -and -not $Keep) {
        Remove-Item -LiteralPath $W -Recurse -Force -ErrorAction SilentlyContinue
    } else {
        Write-Host "`nsandbox kept at: $W"
    }
}

exit ([int]($script:Failed -ne 0))
