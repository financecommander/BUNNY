param(
    [string]$SourceNode = "sw-gpu-core-01",
    [string]$SourceZone = "us-east1-b",
    [string]$SinkNode = "sw-data-01",
    [string]$SinkZone = "us-east1-b",
    [string]$SinkPath = "/data/mee-canonical/sw-gpu-core-01/checkpoints-archive",
    [string]$SinkUser = "Crypt"
)

$publicKey = gcloud compute ssh $SourceNode --zone $SourceZone --tunnel-through-iap --command "cat ~/.ssh/google_compute_engine.pub"
if (-not $publicKey) {
    throw "Unable to read archive source public key from $SourceNode"
}

$escapedKey = $publicKey.Trim().Replace("'", "'\\''")
$remote = @"
set -euo pipefail
sudo install -d -m 0775 -o ${SinkUser} -g ${SinkUser} '$SinkPath'
sudo install -d -m 0700 -o ${SinkUser} -g ${SinkUser} /home/${SinkUser}/.ssh
touch /home/${SinkUser}/.ssh/authorized_keys
grep -F -- '$escapedKey' /home/${SinkUser}/.ssh/authorized_keys >/dev/null 2>&1 || echo '$escapedKey' >> /home/${SinkUser}/.ssh/authorized_keys
chown ${SinkUser}:${SinkUser} /home/${SinkUser}/.ssh/authorized_keys
chmod 0600 /home/$SinkUser/.ssh/authorized_keys
ls -ld '$SinkPath'
"@

gcloud compute ssh $SinkNode --zone $SinkZone --tunnel-through-iap --command $remote
