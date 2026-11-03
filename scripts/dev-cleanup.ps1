# Keeps agent worktrees and build caches from filling the disk (issue #99).
#
#   powershell -File scripts/dev-cleanup.ps1 [-DryRun] [-IncrementalLimitGB 10]
#
# Runs on every new Claude Code and Codex session (SessionStart in .claude/settings.json and
# .codex/hooks.json). It only removes what can be rebuilt or is already on origin:
# - worktrees whose PR is merged or closed and that have no uncommitted changes, plus detached
#   worktrees whose commit is on origin/main and that git has not touched for a day
#   (never the worktree this runs from),
# - local branches whose PR is merged or closed, via `git branch -d` (refuses unmerged commits),
#   and leftover `worktree-agent-*` branches already contained in origin/main,
# - abandoned `.git/objects/**/tmp_obj_*` files older than a day,
# - `target/debug/incremental` once it grows past the limit.
# Then it lists open PRs that are ready to merge, so the agent can remind the user.
param(
    [switch]$DryRun,
    [int]$IncrementalLimitGB = 10
)
$ErrorActionPreference = 'Continue'

function Get-DirSize([string]$Path) {
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue |
        Measure-Object -Property Length -Sum).Sum
    if ($sum) { [double]$sum } else { 0 }
}

function Invoke-Step([string]$Message, [scriptblock]$Action) {
    if ($DryRun) { "[dry-run] $Message" } else { $null = & $Action; $Message }
}

$commonDir = git rev-parse --path-format=absolute --git-common-dir 2>$null
if (-not $commonDir) { exit 0 }
$root = Split-Path -Parent $commonDir
$here = (git rev-parse --show-toplevel 2>$null) -replace '\\', '/'
Set-Location $root
$report = @()
$cutoff = (Get-Date).AddDays(-1)

git fetch --prune --quiet 2>$null

$gh = [bool](Get-Command gh -ErrorAction SilentlyContinue)
$doneHeads = @()
if ($gh) {
    $doneHeads = @(gh pr list --state merged --limit 200 --json headRefName --jq '.[].headRefName' 2>$null) +
        @(gh pr list --state closed --limit 200 --json headRefName --jq '.[].headRefName' 2>$null)
}

# Worktrees: `git worktree list --porcelain` gives blocks of "worktree <path>" / "branch refs/heads/<name>".
$worktrees = @()
$current = $null
foreach ($line in git worktree list --porcelain) {
    if ($line -like 'worktree *') { $current = @{ Path = $line.Substring(9); Branch = '' }; $worktrees += $current }
    elseif ($line -like 'branch refs/heads/*' -and $current) { $current.Branch = $line.Substring(18) }
}
foreach ($wt in $worktrees | Select-Object -Skip 1) {
    $path = $wt.Path -replace '\\', '/'
    if ($path -eq $here -or -not (Test-Path $path)) { continue }
    if ($wt.Branch) {
        if ($doneHeads -notcontains $wt.Branch) { continue }
        $label = $wt.Branch
    }
    else {
        # Detached worktrees (Codex creates these) have no PR to look at: remove one only when its
        # commit is already on origin/main and git has not touched it for a day.
        git -C $path merge-base --is-ancestor HEAD origin/main 2>$null
        if ($LASTEXITCODE -ne 0) { continue }
        $index = Join-Path (git -C $path rev-parse --absolute-git-dir 2>$null) 'index'
        if ((Test-Path $index) -and (Get-Item $index).LastWriteTime -gt $cutoff) { continue }
        $label = 'detached, on main, idle for a day'
    }
    if (git -C $path status --porcelain 2>$null) {
        $report += "kept worktree $path ($label): it has uncommitted changes"
        continue
    }
    $report += Invoke-Step "removed worktree $path ($label)" { git worktree remove --force $path 2>$null }
}
if (-not $DryRun) { git worktree prune 2>$null }

# Branches still checked out somewhere must stay.
$checkedOut = @(git worktree list --porcelain | Where-Object { $_ -like 'branch refs/heads/*' } |
    ForEach-Object { $_.Substring(18) })
foreach ($b in git for-each-ref --format='%(refname:short)' refs/heads) {
    if ($b -eq 'main' -or $checkedOut -contains $b) { continue }
    $leftover = $false
    if ($b -like 'worktree-agent-*') {
        git merge-base --is-ancestor $b origin/main 2>$null
        $leftover = $LASTEXITCODE -eq 0
    }
    if (-not ($doneHeads -contains $b -or $leftover)) { continue }
    if ($DryRun) { $report += "[dry-run] delete branch $b"; continue }
    $out = git branch -d $b 2>&1
    if ($LASTEXITCODE -eq 0) { $report += "deleted branch $b" }
    else { $report += "kept branch ${b}: it has commits that are not on main (check before deleting)" }
}

$tmp =@(Get-ChildItem (Join-Path $commonDir 'objects') -Recurse -File -Force -Filter 'tmp_obj_*' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -lt $cutoff })
if ($tmp.Count -gt 0) {
    $gb = ($tmp | Measure-Object -Property Length -Sum).Sum / 1GB
    $report += Invoke-Step ("deleted {0} abandoned git temp objects ({1:N1} GB)" -f $tmp.Count, $gb) {
        $tmp | Remove-Item -Force -ErrorAction SilentlyContinue
    }
}

$incremental = Join-Path $root 'target/debug/incremental'
if (Test-Path $incremental) {
    $size = Get-DirSize $incremental
    if ($size -gt $IncrementalLimitGB * 1GB) {
        $report += Invoke-Step ("deleted target/debug/incremental ({0:N1} GB)" -f ($size / 1GB)) {
            Remove-Item -LiteralPath $incremental -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

if ($gh) {
    $ready = @(gh pr list --state open --json number,title,mergeStateStatus,isDraft `
            --jq '.[] | select(.isDraft | not) | select(.mergeStateStatus == "CLEAN") | "#\(.number) \(.title)"' 2>$null)
    if ($ready.Count -gt 0) {
        $report += 'Open PRs with green checks waiting for a merge (remind the user; merge only when they say so):'
        $report += $ready | ForEach-Object { "  $_" }
    }
}

if ($report.Count -gt 0) {
    'hece dev-cleanup:'
    $report
}
exit 0
