param([string] $FilePath, [switch] $PreflightOnly, [switch] $VerifyOnly)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Authenticode signing requires Windows' }
$thumbprint = $env:WINDOWS_SIGNING_CERTIFICATE_SHA1
if ($thumbprint -notmatch '^[0-9a-fA-F]{40}$') { throw 'Set WINDOWS_SIGNING_CERTIFICATE_SHA1 to the exact signing identity' }
$thumbprint = $thumbprint.ToUpperInvariant()
$certificate = Get-Item "Cert:\CurrentUser\My\$thumbprint" -ErrorAction Stop
if (-not $VerifyOnly) {
    if (-not $certificate.HasPrivateKey -or $certificate.NotAfter -le (Get-Date) -or $certificate.NotBefore -gt (Get-Date)) { throw 'The signing identity is expired, not yet valid or has no private key' }
    if (-not ($certificate.EnhancedKeyUsageList | Where-Object { $_.ObjectId -eq '1.3.6.1.5.5.7.3.3' })) { throw 'The certificate is not a code-signing identity' }
    $chain = New-Object System.Security.Cryptography.X509Certificates.X509Chain
    try {
        if (-not $chain.Build($certificate) -or $certificate.Subject -eq $certificate.Issuer) { throw 'A publicly trusted code-signing certificate is required' }
    } finally { $chain.Dispose() }
}
if ($PreflightOnly) { Write-Output 'Signing identity preflight passed'; exit 0 }
if (-not $FilePath -or -not (Test-Path -LiteralPath $FilePath -PathType Leaf)) { throw 'Select an existing executable to sign or verify' }
if (-not $VerifyOnly) {
    $timestamp = if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { 'https://timestamp.digicert.com' }
    $uri = [Uri] $timestamp
    if ($uri.Scheme -notin @('https', 'http') -or -not $uri.Host -or $uri.UserInfo) { throw 'Invalid RFC 3161 timestamp URL' }
    $tool = if ($env:WINDOWS_SIGNTOOL) { $env:WINDOWS_SIGNTOOL } else {
        (Get-Item "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" | Sort-Object { [Version] $_.Directory.Parent.Name } -Descending | Select-Object -First 1).FullName
    }
    if (-not $tool -or -not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw 'Windows SDK signtool.exe was not found' }
    & $tool sign /sha1 $thumbprint /fd SHA256 /tr $timestamp /td SHA256 $FilePath
    if ($LASTEXITCODE -ne 0) { throw 'SignTool signing or timestamping failed' }
}
$signature = Get-AuthenticodeSignature -LiteralPath $FilePath
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $thumbprint -or -not $signature.TimeStamperCertificate) { throw 'The executable must have a valid signature from the selected publisher and an RFC 3161 timestamp' }
Write-Output ('Verified signed executable: ' + [IO.Path]::GetFileName($FilePath))
