#Requires -Version 5.1
<#
.SYNOPSIS
  Release zcode-claimer: bump version -> commit -> push -> tag -> CI tu dong build + release.
  Chay tren Windows, tai thu muc goc cua repo.

.DESCRIPTION
  Script lam tat ca cac buoc thu cong khi release mot phien ban moi:
    1. Doi version o dung 5 cho: package.json, src-tauri/Cargo.toml,
       src-tauri/tauri.conf.json, src/index.html (brand), src/main.js (appVersionText)
    2. Chay `pnpm run build` de kiem tra frontend (bo qua voi -SkipBuild)
    3. Commit + push len main
    4. Gan tag v<version> + push tag -> workflow Build Windows x64 tren GitHub
       tu dong build .msi/.exe, tao GitHub Release va cap nhat latest.json

  Khong can chay `pnpm tauri build` o may local nua (tru khi muon test tay).

.EXAMPLE
  .\scripts\release.ps1 -Version 0.1.12
  .\scripts\release.ps1 -Bump patch        # 0.1.11 -> 0.1.12
  .\scripts\release.ps1 -Bump minor        # 0.1.11 -> 0.2.0
  .\scripts\release.ps1 -Version 0.2.0 -SkipBuild
  .\scripts\release.ps1 -Version 0.1.12 -DryRun   # chi in cac buoc, khong lam gi

.NOTES
  Yeu cau: git, node + pnpm (neu khong -SkipBuild). Branch hien tai phai la main,
  cay git phai sach (tru khi -AllowDirty) va main phai dang o trang thai moi nhat
  so voi origin/main.
#>
param(
  [string]$Version,
  [ValidateSet("patch", "minor", "major")][string]$Bump,
  [switch]$SkipBuild,
  [switch]$AllowDirty,
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------------------
# Ham phu
# ---------------------------------------------------------------------------
function Run-Native {
  param([string]$Exe, [string[]]$Arguments, [string]$What)
  Write-Host "  $ $Exe $($Arguments -join ' ')"
  if ($DryRun) { return }
  & $Exe @Arguments
  if ($LASTEXITCODE -ne 0) { throw "$What that bai (exit code $LASTEXITCODE)" }
}

function Get-GitOutput {
  param([string[]]$Arguments)
  $out = & git @Arguments 2>$null
  return ($out | Out-String).Trim()
}

function Set-VersionInFile {
  # Thay dung 1 cho khop $Pattern trong $Path bang $NewVersion (giua group 1 va 2).
  # Ghi file UTF-8 KHONG BOM de khong tao diff thua.
  param([string]$Path, [string]$Pattern, [string]$NewVersion)
  $text = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
  $matches = [regex]::Matches($text, $Pattern)
  if ($matches.Count -ne 1) {
    throw "$Path : tim thay $($matches.Count) cho khop voi pattern, can dung 1 cho. Dung script, kiem tra tay."
  }
  $newText = [regex]::Replace($text, $Pattern, {
      param($m) $m.Groups[1].Value + $NewVersion + $m.Groups[2].Value
    })
  if ($DryRun) {
    $old = $matches[0].Value
    $new = $matches[0].Groups[1].Value + $NewVersion + $matches[0].Groups[2].Value
    Write-Host "  [dry-run] $Path : $old -> $new"
    return
  }
  [System.IO.File]::WriteAllText($Path, $newText, (New-Object System.Text.UTF8Encoding($false)))
}

function Confirm-VersionInFile {
  # Sau khi doi, file phai chua dung 1 lan version moi, 0 lan version cu.
  param([string]$Path, [string]$OldVersion, [string]$NewVersion)
  $text = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
  $nNew = ([regex]::Matches($text, [regex]::Escape($NewVersion))).Count
  $nOld = ([regex]::Matches($text, [regex]::Escape($OldVersion))).Count
  if ($DryRun) { return }
  if ($nNew -lt 1 -or $nOld -ne 0) {
    throw "$Path : kiem tra that bai (moi=$nNew, cu=$nOld). Dung script, kiem tra tay."
  }
}

# ---------------------------------------------------------------------------
# 0. Ve thu muc goc repo
# ---------------------------------------------------------------------------
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot
Write-Host "Repo: $repoRoot"

# ---------------------------------------------------------------------------
# 1. Xac dinh version moi
# ---------------------------------------------------------------------------
$currentVersion = (Get-Content package.json -Raw -Encoding UTF8 | ConvertFrom-Json).version
if (-not $currentVersion -or $currentVersion -notmatch '^\d+\.\d+\.\d+$') {
  throw "Khong doc duoc version hien tai tu package.json (got: '$currentVersion')"
}

if ($Version) {
  if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "-Version phai dang x.y.z, vd 0.1.12 (got: '$Version')"
  }
  $newVersion = $Version
}
elseif ($Bump) {
  $parts = $currentVersion.Split('.') | ForEach-Object { [int]$_ }
  switch ($Bump) {
    "patch" { $parts[2]++ }
    "minor" { $parts[1]++; $parts[2] = 0 }
    "major" { $parts[0]++; $parts[1] = 0; $parts[2] = 0 }
  }
  $newVersion = ($parts -join '.')
}
else {
  throw "Can -Version x.y.z hoac -Bump patch|minor|major. Vd: .\scripts\release.ps1 -Bump patch"
}

if ($newVersion -eq $currentVersion) {
  throw "Version moi ($newVersion) trung version hien tai ($currentVersion). Dung script."
}
Write-Host "Bump version: $currentVersion -> $newVersion"

# ---------------------------------------------------------------------------
# 2. Kiem tra dieu kien truoc khi lam
# ---------------------------------------------------------------------------
$branch = Get-GitOutput @("branch", "--show-current")
if ($branch -ne "main") {
  throw "Branch hien tai la '$branch', release chi lam tu main. Checkout main roi chay lai."
}

if (-not $DryRun) {
  Run-Native git @("fetch", "origin") "git fetch origin"
  $behind = [int](Get-GitOutput @("rev-list", "--count", "HEAD..origin/main"))
  if ($behind -gt 0) {
    throw "main local dang sau origin/main $behind commit. Pull ve truoc roi chay lai."
  }
  $tagExistsLocal = & git rev-parse "v$newVersion" 2>$null
  $tagExistsRemote = Get-GitOutput @("ls-remote", "--tags", "origin", "v$newVersion")
  if ($tagExistsLocal -or $tagExistsRemote) {
    throw "Tag v$newVersion da ton tai (local hoac remote). Chon version khac."
  }
  $dirty = Get-GitOutput @("status", "--porcelain")
  if ($dirty -and -not $AllowDirty) {
    throw "Cay git dang ban (co file chua commit). Commit/stash truoc, hoac chay voi -AllowDirty."
  }
}
else {
  Write-Host "[dry-run] Bo qua kiem tra git (fetch, tag, clean tree)."
}

# ---------------------------------------------------------------------------
# 3. Doi version o 5 cho (moi cho dung 1 lan khop)
# ---------------------------------------------------------------------------
$versionFiles = @(
  @{ Path = "package.json";            Pattern = "(`"version`":\s*`")$([regex]::Escape($currentVersion))(`")" },
  @{ Path = "src-tauri\Cargo.toml";    Pattern = "(?m)^(version\s*=\s*`")$([regex]::Escape($currentVersion))(`")" },
  @{ Path = "src-tauri\tauri.conf.json"; Pattern = "(`"version`":\s*`")$([regex]::Escape($currentVersion))(`")" },
  @{ Path = "src\index.html";          Pattern = "(>v)$([regex]::Escape($currentVersion))( &middot; Windows<)" },
  @{ Path = "src\main.js";             Pattern = "(let appVersionText = `"v)$([regex]::Escape($currentVersion))(`";)" }
)

Write-Host "Doi version o 5 cho:"
$changedFiles = @()
try {
  foreach ($f in $versionFiles) {
    Set-VersionInFile -Path $f.Path -Pattern $f.Pattern -NewVersion $newVersion
    $changedFiles += $f.Path
    Confirm-VersionInFile -Path $f.Path -OldVersion $currentVersion -NewVersion $newVersion
    Write-Host "  OK $($f.Path)"
  }
}
catch {
  if (-not $DryRun) {
    Write-Host "Gap loi, hoan tac cac file da doi..."
    foreach ($p in $changedFiles) { & git checkout -- $p 2>$null }
  }
  throw
}

# ---------------------------------------------------------------------------
# 4. Build frontend de kiem tra (tru khi -SkipBuild)
# ---------------------------------------------------------------------------
if (-not $SkipBuild) {
  Write-Host "Kiem tra build frontend (pnpm run build):"
  Run-Native pnpm @("run", "build") "pnpm run build"
}
else {
  Write-Host "Bo qua build frontend (-SkipBuild)."
}

if ($DryRun) {
  Write-Host ""
  Write-Host "[dry-run] Se commit + push + tag v$newVersion (khong thuc hien)."
  Write-Host "[dry-run] Xong. Chay lai khong co -DryRun de release that."
  exit 0
}

# ---------------------------------------------------------------------------
# 5. Commit + push main
# ---------------------------------------------------------------------------
Write-Host "Commit + push main:"
Run-Native git @("add", "package.json", "src-tauri\Cargo.toml", "src-tauri\tauri.conf.json", "src\index.html", "src\main.js") "git add"
Run-Native git @("commit", "-m", "chore: bump version $currentVersion -> $newVersion") "git commit"
Run-Native git @("push", "origin", "main") "git push origin main"

# ---------------------------------------------------------------------------
# 6. Tag + push tag -> CI tu dong build + release
# ---------------------------------------------------------------------------
Write-Host "Gan tag v$newVersion + push tag (CI se tu dong build):"
Run-Native git @("tag", "v$newVersion") "git tag v$newVersion"
Run-Native git @("push", "origin", "v$newVersion") "git push origin v$newVersion"

Write-Host ""
Write-Host "Xong! Da release v$newVersion."
Write-Host "  Release: https://github.com/ngapngap/zcode-claimer/releases/tag/v$newVersion"
Write-Host "  CI chay: https://github.com/ngapngap/zcode-claimer/actions"
Write-Host "  CI mat vai phut de build .msi/.exe roi dang len Release. App se tu thay ban moi qua updater (latest.json)."
