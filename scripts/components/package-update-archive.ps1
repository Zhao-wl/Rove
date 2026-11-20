param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Destination,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

# 手动包保留版本目录。旧更新器在自己的暂存目录内解包，要求清单在 ZIP 根。
$prefix = "$Version/"
$temporary = "$Destination.$([Guid]::NewGuid().ToString('N')).tmp"
try {
    $inputArchive = [IO.Compression.ZipFile]::OpenRead($Source)
    try {
        $outputArchive = [IO.Compression.ZipFile]::Open(
            $temporary, [IO.Compression.ZipArchiveMode]::Create
        )
        try {
            $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
            $hasManifest = $false
            $hasExe = $false
            foreach ($entry in $inputArchive.Entries) {
                $name = $entry.FullName.Replace('\', '/')
                if ($name -ceq $prefix) { continue }
                if (!$name.StartsWith($prefix, [StringComparison]::Ordinal)) {
                    throw "Unexpected component archive entry: $name"
                }
                $relative = $name.Substring($prefix.Length)
                $isDirectory = $relative.EndsWith('/')
                $clean = $relative.TrimEnd('/')
                if (!$clean -or $relative.StartsWith('/') -or $relative.Contains('//') -or
                    $clean.Contains(':') -or $clean.IndexOf([char]0) -ge 0 -or
                    ($clean.Split('/') | Where-Object { $_ -in '.', '..' }) -or !$names.Add($clean)) {
                    throw "Invalid component archive entry: $name"
                }

                $newEntry = $outputArchive.CreateEntry($relative, [IO.Compression.CompressionLevel]::Optimal)
                $newEntry.LastWriteTime = $entry.LastWriteTime
                if (!$isDirectory) {
                    $sourceStream = $entry.Open()
                    try {
                        $destinationStream = $newEntry.Open()
                        try { $sourceStream.CopyTo($destinationStream) }
                        finally { $destinationStream.Dispose() }
                    } finally { $sourceStream.Dispose() }
                    if ($relative -ceq 'component.json') {
                        $reader = [IO.StreamReader]::new($entry.Open())
                        try { $metadata = $reader.ReadToEnd() | ConvertFrom-Json }
                        finally { $reader.Dispose() }
                        if ($metadata.component -cne 'indexer-cs' -or $metadata.version -cne $Version) {
                            throw "Component manifest must report indexer-cs $Version"
                        }
                        $hasManifest = $true
                    }
                    if ($relative -ceq 'rove-indexer-cs.exe') { $hasExe = $true }
                }
            }
            if (!$hasManifest -or !$hasExe) { throw "Component ZIP needs component.json and rove-indexer-cs.exe in $prefix" }
        } finally { $outputArchive.Dispose() }
    } finally { $inputArchive.Dispose() }
    Move-Item -LiteralPath $temporary -Destination $Destination -Force
} finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
}
