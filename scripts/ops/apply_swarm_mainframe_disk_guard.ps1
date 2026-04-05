param(
    [string]$RemoteHost = 'crypticassassin@34.148.140.31',
    [string]$SshKeyPath = 'C:\Users\Crypt\.ssh\google_compute_engine'
)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$sshOptions = @(
    '-i', $SshKeyPath,
    '-o', 'StrictHostKeyChecking=no',
    '-o', 'BatchMode=yes'
)

& ssh @sshOptions $RemoteHost 'sudo mkdir -p /srv/swarm/repo'
if ($LASTEXITCODE -ne 0) { throw 'failed to prepare mainframe repo dir' }

$archiveCommand = "tar -cf - -C `"$repoRoot`" scripts/swarm_mee_cleanup.sh scripts/ops/mee_checkpoint_retention.py infra/logrotate/swarm-mainframe.conf infra/systemd/swarm-mainframe-disk-guard.service infra/systemd/swarm-mainframe-disk-guard.timer infra/provisioning/linux/install_mainframe_disk_guard.sh | ssh -i `"$SshKeyPath`" -o StrictHostKeyChecking=no -o BatchMode=yes $RemoteHost `"rm -rf /tmp/swarm-mainframe-disk-guard && mkdir -p /tmp/swarm-mainframe-disk-guard && cd /tmp/swarm-mainframe-disk-guard && tar -xf -`""
cmd /c $archiveCommand
if ($LASTEXITCODE -ne 0) { throw 'failed to transfer mainframe disk guard payload' }

$remote = @'
set -e
sudo mkdir -p /srv/swarm/repo/scripts/ops /srv/swarm/repo/infra/logrotate /srv/swarm/repo/infra/systemd /srv/swarm/repo/infra/provisioning/linux
sudo rsync -a /tmp/swarm-mainframe-disk-guard/scripts/ /srv/swarm/repo/scripts/
sudo rsync -a /tmp/swarm-mainframe-disk-guard/infra/ /srv/swarm/repo/infra/
sudo bash /srv/swarm/repo/infra/provisioning/linux/install_mainframe_disk_guard.sh /srv/swarm/repo /srv/swarm/apps/swarm-mainframe
sudo systemctl start swarm-mainframe-disk-guard.service
sudo systemctl is-enabled swarm-mainframe-disk-guard.timer
sudo systemctl is-active swarm-mainframe-disk-guard.timer
'@

& ssh @sshOptions $RemoteHost $remote
if ($LASTEXITCODE -ne 0) { throw 'failed to install mainframe disk guard' }

Write-Host 'Mainframe disk guard apply complete.'
