function Install-Devzymer {
    $ErrorActionPreference = 'Stop'

    $repo = if ($env:DEVZYMER_REPO) { $env:DEVZYMER_REPO } else { 'devadeiltonlima/devzymer' }
    $version = if ($env:DEVZYMER_VERSION) { $env:DEVZYMER_VERSION } else { 'latest' }
    $installDir = if ($env:DEVZYMER_INSTALL_DIR) { $env:DEVZYMER_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'devzymer\bin' }

    if ($env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'ARM64')) {
        throw "devzymer: arquitetura nao suportada: $($env:PROCESSOR_ARCHITECTURE)"
    }
    $archive = 'devzymer-x64-pc-windows-msvc.zip'

    if ($env:DEVZYMER_BASE_URL) {
        $base = $env:DEVZYMER_BASE_URL
    }
    elseif ($version -eq 'latest') {
        $base = "https://github.com/$repo/releases/latest/download"
    }
    else {
        $base = "https://github.com/$repo/releases/download/$version"
    }

    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("devzymer-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null

    try {
        $client = New-Object System.Net.WebClient
        $zipPath = Join-Path $tmp $archive
        $sumPath = "$zipPath.sha256"

        Write-Host "devzymer: baixando $archive"
        try {
            $client.DownloadFile("$base/$archive", $zipPath)
            $client.DownloadFile("$base/$archive.sha256", $sumPath)
        }
        catch {
            throw "devzymer: nao foi possivel baixar $base/$archive ($($_.Exception.Message))"
        }

        $expected = ((Get-Content $sumPath -TotalCount 1) -split '\s+')[0].ToLower()
        $actual = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
        if (-not $expected -or $expected -ne $actual) {
            throw 'devzymer: o checksum do download nao confere; nada foi instalado'
        }

        $extracted = Join-Path $tmp 'extracted'
        Expand-Archive -Path $zipPath -DestinationPath $extracted -Force
        New-Item -ItemType Directory -Path $installDir -Force | Out-Null
        $target = Join-Path $installDir 'devzymer.exe'
        try {
            Copy-Item (Join-Path $extracted 'devzymer.exe') $target -Force
        }
        catch {
            throw "devzymer: nao foi possivel gravar $target. Feche os agentes que estejam usando o devzymer e tente de novo"
        }

        Write-Host "devzymer: instalado em $target"
        & $target --version

        if (-not $env:DEVZYMER_NO_MODIFY_PATH) {
            $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
            $entries = if ($userPath) { $userPath -split ';' } else { @() }
            if ($entries -notcontains $installDir) {
                $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
                [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
                Write-Host "devzymer: $installDir adicionado ao PATH do usuario"
            }
            if (($env:Path -split ';') -notcontains $installDir) {
                $env:Path = "$env:Path;$installDir"
            }
        }

        Write-Host ''
        Write-Host 'Proximo passo, dentro do seu projeto:'
        Write-Host '  devzymer init'
    }
    finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Devzymer
