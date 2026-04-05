param(
    [int]$WarnDiskPercent = 75,
    [int]$CriticalDiskPercent = 85,
    [switch]$AsJson
)

$ErrorActionPreference = 'Stop'
$sshKey = 'C:/Users/Crypt/.ssh/google_compute_engine'
$sshOptions = @(
    '-i', $sshKey,
    '-o', 'StrictHostKeyChecking=no',
    '-o', 'BatchMode=yes',
    '-o', 'ConnectTimeout=15'
)

$targets = @(
    @{
        Name = 'sw-mainframe-01'
        Host = 'crypticassassin@34.148.140.31'
        Kind = 'mainframe'
    },
    @{
        Name = 'sw-gpu-mee-01'
        Host = 'crypticassassin@35.231.136.68'
        Kind = 'mee'
    },
    @{
        Name = 'sw-gpu-mee-02'
        Host = 'crypticassassin@34.75.191.94'
        Kind = 'mee'
    }
)

$results = foreach ($target in $targets) {
    if ($target.Kind -eq 'mainframe') {
        $remote = @'
echo node=$(hostname)
echo disk_percent=$(df --output=pcent / | tail -1 | tr -dc '0-9')
disk_guard_timer=$(systemctl is-enabled swarm-mainframe-disk-guard.timer 2>/dev/null || true)
disk_guard_active=$(systemctl is-active swarm-mainframe-disk-guard.timer 2>/dev/null || true)
echo disk_guard_timer=${disk_guard_timer:-missing}
echo disk_guard_active=${disk_guard_active:-missing}
'@
    } else {
        $remote = @'
echo node=$(hostname)
mee_service_enabled=$(systemctl is-enabled mee-gpu-worker.service 2>/dev/null || true)
mee_service_active=$(systemctl is-active mee-gpu-worker.service 2>/dev/null || true)
mee_process_count=$(ps -eo args | grep -F '/srv/swarm/apps/mee/mee_scheduler.py' | grep -v grep | wc -l | tr -d ' ')
echo mee_service_enabled=${mee_service_enabled:-missing}
echo mee_service_active=${mee_service_active:-missing}
echo mee_process_count=${mee_process_count:-0}
'@
    }

    $output = & ssh @sshOptions $target.Host $remote 2>&1
    $lines = @($output | Where-Object { $_ -ne $null }) | ForEach-Object { "$_".Trim() } | Where-Object { $_ -ne '' }
    $data = [ordered]@{ name = $target.Name; kind = $target.Kind; ok = $LASTEXITCODE -eq 0; issues = @() }
    foreach ($line in $lines) {
        if ($line -match '^[a-z_]+=') {
            $parts = $line -split '=', 2
            $data[$parts[0]] = $parts[1]
        }
    }
    $issues = New-Object System.Collections.Generic.List[string]
    if (-not $data.ok) { $issues.Add('transport-failed') }
    if ($target.Kind -eq 'mainframe') {
        $disk = if ($data.disk_percent) { [int]$data.disk_percent } else { -1 }
        if ($disk -ge $CriticalDiskPercent) { $issues.Add("disk-critical=$disk%") }
        elseif ($disk -ge $WarnDiskPercent) { $issues.Add("disk-warn=$disk%") }
        if ($data.disk_guard_timer -notin @('enabled', 'enabled-runtime')) { $issues.Add("disk-guard-timer=$($data.disk_guard_timer)") }
        if ($data.disk_guard_active -ne 'active') { $issues.Add("disk-guard-active=$($data.disk_guard_active)") }
    } else {
        if ($data.mee_service_enabled -notin @('enabled', 'enabled-runtime')) { $issues.Add("mee-enabled=$($data.mee_service_enabled)") }
        if ($data.mee_service_active -ne 'active') { $issues.Add("mee-active=$($data.mee_service_active)") }
        $procCount = if ($data.mee_process_count) { [int]$data.mee_process_count } else { 0 }
        if ($procCount -lt 1) { $issues.Add('mee-process-missing') }
    }

    [pscustomobject]@{
        Name = $target.Name
        Kind = $target.Kind
        DiskPercent = if ($data.disk_percent) { [int]$data.disk_percent } else { $null }
        DiskGuardTimer = $data.disk_guard_timer
        DiskGuardActive = $data.disk_guard_active
        MeeServiceEnabled = $data.mee_service_enabled
        MeeServiceActive = $data.mee_service_active
        MeeProcessCount = if ($data.mee_process_count) { [int]$data.mee_process_count } else { $null }
        Healthy = $issues.Count -eq 0
        Issues = @($issues)
    }
}

if ($AsJson) {
    $results | ConvertTo-Json -Depth 5
    exit (($results | Where-Object { -not $_.Healthy }).Count)
}

$results | Format-Table Name, Kind, DiskPercent, DiskGuardTimer, DiskGuardActive, MeeServiceEnabled, MeeServiceActive, MeeProcessCount, Healthy -AutoSize

$alerts = $results | Where-Object { -not $_.Healthy }
if ($alerts) {
    ''
    'ALERT'
    foreach ($alert in $alerts) {
        "- $($alert.Name): $($alert.Issues -join ', ')"
    }
    exit 2
}

''
'HEALTHY'
'- Mainframe disk guard timer enabled and active'
'- Mainframe root disk below critical threshold'
'- MEE GPU scheduler services enabled and active on both nodes'
exit 0
