param(
    [string]$SshUser = 'crypticassassin',
    [string]$SshPrivateKeyPath = 'C:\Users\Crypt\.ssh\google_compute_engine'
)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$targets = @(
    @{ Name = 'sw-gpu-mee-01'; Host = '35.231.136.68' },
    @{ Name = 'sw-gpu-mee-02'; Host = '34.75.191.94' }
)

foreach ($target in $targets) {
    $remoteHost = "$SshUser@$($target.Host)"
    Write-Host "Applying MEE scheduler supervision on $($target.Name) [$($target.Host)]"
    & ssh -i $SshPrivateKeyPath -o StrictHostKeyChecking=no -o BatchMode=yes $remoteHost "sudo mkdir -p /srv/swarm/repo/infra/systemd"
    if ($LASTEXITCODE -ne 0) { throw "Failed to prepare $($target.Name)" }

    $archiveCommand = "tar -cf - -C `"$repoRoot`" infra/systemd/mee-gpu-worker.service | ssh -i `"$SshPrivateKeyPath`" -o StrictHostKeyChecking=no -o BatchMode=yes $remoteHost `"rm -rf /tmp/mee-supervision && mkdir -p /tmp/mee-supervision && cd /tmp/mee-supervision && tar -xf -`""
    cmd /c $archiveCommand
    if ($LASTEXITCODE -ne 0) { throw "Failed to transfer MEE unit to $($target.Name)" }

    $remote = @'
set -e
sudo rsync -a /tmp/mee-supervision/infra/systemd/ /srv/swarm/repo/infra/systemd/
sudo install -m 0644 /srv/swarm/repo/infra/systemd/mee-gpu-worker.service /etc/systemd/system/mee-gpu-worker.service
sudo systemctl daemon-reload
sudo systemctl unmask mee-gpu-worker.service || true
sudo systemctl enable mee-gpu-worker.service
sudo systemctl restart mee-gpu-worker.service
sleep 3
sudo systemctl is-active mee-gpu-worker.service
pgrep -af mee_scheduler.py
'@
    & ssh -i $SshPrivateKeyPath -o StrictHostKeyChecking=no -o BatchMode=yes $remoteHost $remote
    if ($LASTEXITCODE -ne 0) { throw "Failed to activate MEE scheduler supervision on $($target.Name)" }
}

Write-Host 'MEE GPU scheduler supervision apply complete.'
