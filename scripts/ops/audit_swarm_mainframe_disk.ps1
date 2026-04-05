param(
    [string]$RemoteHost = 'crypticassassin@34.148.140.31',
    [string]$SshKeyPath = 'C:\Users\Crypt\.ssh\google_compute_engine'
)

$ErrorActionPreference = 'Stop'

$sshOptions = @(
    '-i', $SshKeyPath,
    '-o', 'StrictHostKeyChecking=no',
    '-o', 'BatchMode=yes'
)

$remote = @'
set -e
echo "== root =="
df -h /
echo
echo "== largest dirs =="
sudo du -x -h --max-depth=2 /srv /opt /var 2>/dev/null | sort -h | tail -n 40
echo
echo "== mainframe model research =="
sudo du -x -h --max-depth=2 /srv/swarm/apps/swarm-mainframe/model_research 2>/dev/null | sort -h | tail -n 40
echo
echo "== biggest log files =="
sudo find /srv/swarm/logs -type f -printf '%s %TY-%Tm-%Td %TT %p\n' 2>/dev/null | sort -nr | head -n 30
echo
echo "== journal usage =="
sudo journalctl --disk-usage || true
echo
echo "== docker usage =="
sudo docker system df || true
'@

& ssh @sshOptions $RemoteHost $remote
if ($LASTEXITCODE -ne 0) {
    throw 'mainframe disk audit failed'
}
