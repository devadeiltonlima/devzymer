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

    $esc = [char]27
    $redirected = [Console]::IsOutputRedirected
    $forced = [bool]$env:DEVZYMER_FORCE_FANCY
    $vt = $false
    try { $vt = [bool]$Host.UI.SupportsVirtualTerminal } catch { $vt = $false }
    if ($PSVersionTable.PSVersion.Major -ge 7 -and -not $redirected) { $vt = $true }
    $fancy = (-not $env:NO_COLOR) -and ($forced -or ((-not $redirected) -and $vt))
    $animate = $fancy -and (-not $redirected) -and (-not $env:DEVZYMER_NO_ANIM) -and (-not $env:CI)
    $unicode = $forced -or [bool]$env:WT_SESSION -or [bool]$env:TERM_PROGRAM -or ($env:ConEmuANSI -eq 'ON')

    $previousEncoding = $null
    if ($fancy) {
        try {
            $previousEncoding = [Console]::OutputEncoding
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
        }
        catch { $previousEncoding = $null }
    }

    $stops = @(@(0, 245, 160), @(0, 217, 245), @(124, 77, 255))
    $muted = @(139, 148, 158)
    $good = @(63, 185, 80)
    $bad = @(248, 81, 73)
    $white = @(230, 237, 243)
    $background = @(13, 17, 23)

    function Mix($a, $b, [double]$t) {
        return @(
            [int][Math]::Round($a[0] + ($b[0] - $a[0]) * $t),
            [int][Math]::Round($a[1] + ($b[1] - $a[1]) * $t),
            [int][Math]::Round($a[2] + ($b[2] - $a[2]) * $t)
        )
    }

    function Gradient([double]$t) {
        $t = [Math]::Max(0.0, [Math]::Min(1.0, $t)) * 2
        if ($t -le 1) { return Mix $stops[0] $stops[1] $t }
        return Mix $stops[1] $stops[2] ($t - 1)
    }

    function Paint($rgb, [string]$text) {
        return "$esc[38;2;$($rgb[0]);$($rgb[1]);$($rgb[2])m$text$esc[0m"
    }

    function Out-Raw([string]$text) {
        [Console]::Out.Write($text)
    }

    $bannerBase64 = 'IOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiCAgIOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiCDilojiloji' +
        'lojilojiloggICDilojilojilojilojilogg4paI4paI4paI4paI4paI4paI4paI4paI4paI4paI4paIIOKWiOKWiOKWiOKW' +
        'iOKWiCDilojilojilojilojilogg4paI4paI4paI4paI4paI4paIICAg4paI4paI4paI4paI4paI4paIIOKWiOKWiOKWiOKW' +
        'iOKWiOKWiOKWiOKWiOKWiOKWiCDilojilojilojilojilojilojilojilojilojilojilogK4paS4paS4paI4paI4paI4paS' +
        '4paS4paS4paS4paI4paI4paIIOKWkuKWkuKWiOKWiOKWiOKWkuKWkuKWkuKWkuKWkuKWiOKWkuKWkuKWiOKWiOKWiCAgIOKW' +
        'kuKWkuKWiOKWiOKWiCDilpLilojilpLilpLilpLilpLilpLilpLilojilojilogg4paS4paS4paI4paI4paIIOKWkuKWkuKW' +
        'iOKWiOKWiCDilpLilpLilojilojilojilojilojilogg4paI4paI4paI4paI4paI4paIIOKWkuKWkuKWiOKWiOKWiOKWkuKW' +
        'kuKWkuKWkuKWkuKWiOKWkuKWkuKWiOKWiOKWiOKWkuKWkuKWkuKWkuKWkuKWiOKWiOKWiAog4paS4paI4paI4paIICAg4paS' +
        '4paS4paI4paI4paIIOKWkuKWiOKWiOKWiCAg4paIIOKWkiAg4paS4paI4paI4paIICAgIOKWkuKWiOKWiOKWiCDilpIgICAg' +
        'IOKWiOKWiOKWiOKWkiAgIOKWkuKWkuKWiOKWiOKWiCDilojilojiloggICDilpLilojilojilojilpLilojilojilojiloji' +
        'lojilpLilojilojiloggIOKWkuKWiOKWiOKWiCAg4paIIOKWkiAg4paS4paI4paI4paIICAgIOKWkuKWiOKWiOKWiAog4paS' +
        '4paI4paI4paIICAgIOKWkuKWiOKWiOKWiCDilpLilojilojilojilojilojiloggICAg4paS4paI4paI4paIICAgIOKWkuKW' +
        'iOKWiOKWiCAgICAgIOKWiOKWiOKWiCAgICAgIOKWkuKWkuKWiOKWiOKWiOKWiOKWiCAgICDilpLilojilojilojilpLilpLi' +
        'lojilojilogg4paS4paI4paI4paIICDilpLilojilojilojilojilojiloggICAg4paS4paI4paI4paI4paI4paI4paI4paI' +
        '4paI4paI4paICiDilpLilojilojiloggICAg4paS4paI4paI4paIIOKWkuKWiOKWiOKWiOKWkuKWkuKWiCAgICDilpLilpLi' +
        'lojilojiloggICDilojilojiloggICAgICDilojilojiloggICAgICAgIOKWkuKWkuKWiOKWiOKWiCAgICAg4paS4paI4paI' +
        '4paIIOKWkuKWkuKWkiAg4paS4paI4paI4paIICDilpLilojilojilojilpLilpLiloggICAg4paS4paI4paI4paI4paS4paS' +
        '4paS4paS4paS4paI4paI4paICiDilpLilojilojiloggICAg4paI4paI4paIICDilpLilojilojilogg4paSICAg4paIICDi' +
        'lpLilpLilpLilojilojilojilojilojilpIgICAgIOKWiOKWiOKWiOKWiCAgICAg4paIICAgIOKWkuKWiOKWiOKWiCAgICAg' +
        '4paS4paI4paI4paIICAgICAg4paS4paI4paI4paIICDilpLilojilojilogg4paSICAg4paIIOKWkuKWiOKWiOKWiCAgICDi' +
        'lpLilojilojilogKIOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiCAgIOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKW' +
        'iOKWiCAgICDilpLilpLilojilojiloggICAgICDilojilojilojilojilojilojilojilojilojilojiloggICAg4paI4paI' +
        '4paI4paI4paIICAgIOKWiOKWiOKWiOKWiOKWiCAgICAg4paI4paI4paI4paI4paIIOKWiOKWiOKWiOKWiOKWiOKWiOKWiOKW' +
        'iOKWiOKWiCDilojilojilojilojiloggICDilojilojilojilojilogK4paS4paS4paS4paS4paS4paS4paS4paS4paS4paS' +
        'ICAg4paS4paS4paS4paS4paS4paS4paS4paS4paS4paSICAgICAg4paS4paS4paSICAgICAg4paS4paS4paS4paS4paS4paS' +
        '4paS4paS4paS4paS4paSICAgIOKWkuKWkuKWkuKWkuKWkiAgICDilpLilpLilpLilpLilpIgICAgIOKWkuKWkuKWkuKWkuKW' +
        'kiDilpLilpLilpLilpLilpLilpLilpLilpLilpLilpIg4paS4paS4paS4paS4paSICAg4paS4paS4paS4paS4paS'

    function Show-Banner {
        $rows = ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($bannerBase64))) -split "`n"
        $width = 103
        $columns = 120
        try { $columns = $Host.UI.RawUI.WindowSize.Width } catch { $columns = 120 }
        if ($columns -lt 105) {
            $letters = 'DEVZYMER'.ToCharArray()
            $line = '  '
            for ($i = 0; $i -lt $letters.Length; $i++) { $line += Paint (Gradient ($i / 7.0)) ("$esc[1m" + $letters[$i]) }
            Out-Raw ("`n$line`n  Less Tokens. More Engineering.`n`n")
            return
        }
        $shadow = [char]0x2592
        $solid = @(); $dim = @()
        for ($c = 0; $c -lt $width; $c++) {
            $g = Gradient ($c / ($width - 1.0))
            $solid += , $g
            $dim += , (Mix $background $g 0.42)
        }
        function Row([string]$row, $center) {
            $sb = New-Object System.Text.StringBuilder
            $previous = ''
            for ($c = 0; $c -lt $row.Length; $c++) {
                $glyph = $row[$c]
                if ($glyph -eq ' ') { [void]$sb.Append(' '); continue }
                $rgb = if ($glyph -eq $shadow) { $dim[$c] } else { $solid[$c] }
                if ($null -ne $center) {
                    $boost = [Math]::Max(0.0, 1.0 - ([Math]::Abs($c - $center) / 9.0))
                    if ($boost -gt 0) { $rgb = Mix $rgb $white ($boost * 0.7) }
                }
                $code = "$esc[38;2;$($rgb[0]);$($rgb[1]);$($rgb[2])m"
                if ($code -ne $previous) { [void]$sb.Append($code); $previous = $code }
                [void]$sb.Append($glyph)
            }
            [void]$sb.Append("$esc[0m")
            return $sb.ToString()
        }
        $rows = $rows | ForEach-Object { $_.PadRight($width) }
        Out-Raw "`n"
        if ($animate) {
            Out-Raw "$esc[?25l"
            foreach ($row in $rows) { Out-Raw ((Row $row $null) + "`n"); Start-Sleep -Milliseconds 55 }
            for ($frame = 0; $frame -lt 12; $frame++) {
                Out-Raw ("$esc[" + $rows.Count + 'A')
                $center = -10 + $frame * 10
                foreach ($row in $rows) { Out-Raw ((Row $row $center) + "`n") }
                Start-Sleep -Milliseconds 45
            }
            Out-Raw ("$esc[" + $rows.Count + 'A')
            foreach ($row in $rows) { Out-Raw ((Row $row $null) + "`n") }
            Out-Raw "$esc[?25h"
        }
        else {
            foreach ($row in $rows) { Out-Raw ((Row $row $null) + "`n") }
        }
        $tag = ' Less Tokens. More Engineering.'
        Out-Raw ((Paint $stops[0] ("$esc[3m" + $tag)) + "`n`n")
    }

    function Step([string]$mark, [string]$label, [string]$detail) {
        $symbol = '+'; $color = $good
        if ($mark -eq 'fail') { $symbol = 'x'; $color = $bad }
        if ($unicode) {
            if ($mark -eq 'fail') { $symbol = [string][char]0x2716 } else { $symbol = [string][char]0x2714 }
        }
        $shown = if ($label.Length -ge 34) { $label + '  ' } else { $label.PadRight(34) }
        Out-Raw ("`r$esc[2K  " + (Paint $color $symbol) + ' ' + $shown + (Paint $muted $detail) + "`n")
    }

    function Draw-Bar([string]$label, [long]$done, [long]$total) {
        $cells = 28
        $ratio = 0.0
        if ($total -gt 0) { $ratio = [Math]::Min(1.0, $done / [double]$total) }
        $filled = [int][Math]::Round($ratio * $cells)
        $fullChar = '#'; $emptyChar = '-'
        if ($unicode) { $fullChar = [string][char]0x2588; $emptyChar = [string][char]0x2591 }
        $bar = New-Object System.Text.StringBuilder
        for ($i = 0; $i -lt $cells; $i++) {
            if ($i -lt $filled) { [void]$bar.Append((Paint (Gradient ($i / ($cells - 1.0))) $fullChar)) }
            else { [void]$bar.Append((Paint $muted $emptyChar)) }
        }
        $size = '{0:N1} / {1:N1} MB' -f ($done / 1MB), ($total / 1MB)
        $percent = '{0,3}%' -f [int][Math]::Round($ratio * 100)
        Out-Raw ("`r$esc[2K  " + $label.PadRight(32) + $bar.ToString() + ' ' + (Paint $muted "$percent  $size"))
    }

    function Get-File([string]$url, [string]$destination, [string]$label) {
        $request = [System.Net.WebRequest]::Create($url)
        $request.CachePolicy = New-Object System.Net.Cache.RequestCachePolicy([System.Net.Cache.RequestCacheLevel]::NoCacheNoStore)
        if ($request -is [System.Net.HttpWebRequest]) { $request.UserAgent = 'devzymer-installer' }
        $response = $request.GetResponse()
        $total = $response.ContentLength
        $done = 0L
        $netStream = $response.GetResponseStream()
        $fileStream = [System.IO.File]::Create($destination)
        try {
            $buffer = New-Object byte[] 65536
            $last = [DateTime]::UtcNow
            while (($read = $netStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $fileStream.Write($buffer, 0, $read)
                $done += $read
                if ($animate -and ([DateTime]::UtcNow - $last).TotalMilliseconds -gt 60) {
                    Draw-Bar $label $done $total
                    $last = [DateTime]::UtcNow
                }
            }
        }
        finally {
            $fileStream.Dispose()
            $netStream.Dispose()
            $response.Close()
        }
        return $done
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("devzymer-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null

    try {
        if ($fancy) {
            Show-Banner
            Out-Raw ("  " + (Paint (Gradient 0.5) ("$esc[1m" + 'Instalando o devzymer')) + "`n`n")
            Step 'ok' 'Plataforma detectada' 'Windows x64'
        }

        $zipPath = Join-Path $tmp $archive
        $sumPath = "$zipPath.sha256"

        if (-not $fancy) { Write-Host "devzymer: baixando $archive" }
        try {
            $bytes = Get-File "$base/$archive" $zipPath 'Baixando o devzymer'
            [void](Get-File "$base/$archive.sha256" $sumPath 'checksum')
        }
        catch {
            if ($fancy) { Step 'fail' 'Baixando o devzymer' 'falhou' }
            throw "devzymer: nao foi possivel baixar $base/$archive ($($_.Exception.Message))"
        }
        if ($fancy) { Step 'ok' 'Baixando o devzymer' ('{0:N1} MB' -f ($bytes / 1MB)) }

        $expected = ((Get-Content $sumPath -TotalCount 1) -split '\s+')[0].ToLower()
        $actual = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
        if (-not $expected -or $expected -ne $actual) {
            if ($fancy) { Step 'fail' 'Conferindo o SHA-256' 'nao confere' }
            throw 'devzymer: o checksum do download nao confere; nada foi instalado'
        }
        if ($fancy) { Step 'ok' 'Conferindo o SHA-256' $actual.Substring(0, 16) }

        $extracted = Join-Path $tmp 'extracted'
        Expand-Archive -Path $zipPath -DestinationPath $extracted -Force
        New-Item -ItemType Directory -Path $installDir -Force | Out-Null
        $target = Join-Path $installDir 'devzymer.exe'
        $source = Join-Path $extracted 'devzymer.exe'
        $replacedRunning = $false
        try {
            Copy-Item $source $target -Force
        }
        catch {
            $stale = "$target.old-" + (Get-Date -Format 'yyyyMMddHHmmss')
            try {
                Move-Item $target $stale -Force
                Copy-Item $source $target -Force
                $replacedRunning = $true
            }
            catch {
                if ($fancy) { Step 'fail' 'Instalando' 'arquivo em uso' }
                throw "devzymer: nao foi possivel gravar $target. Feche os agentes que estejam usando o devzymer e tente de novo"
            }
        }
        Get-ChildItem -Path $installDir -Filter 'devzymer.exe.old-*' -ErrorAction SilentlyContinue |
            ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }

        if ($fancy) {
            Step 'ok' 'Instalando' $target
            $reported = (& $target --version) -join ' '
            Step 'ok' 'Verificando o executavel' $reported
        }
        else {
            Write-Host "devzymer: instalado em $target"
            & $target --version
        }
        if ($replacedRunning) {
            if ($fancy) { Step 'ok' 'Versao anterior em uso' 'substituida; reinicie os agentes' }
            else { Write-Host 'devzymer: a versao anterior estava em uso e foi substituida. Reinicie os agentes (Claude Code, Codex, OpenCode) para usar a nova.' }
        }

        if (-not $env:DEVZYMER_NO_MODIFY_PATH) {
            $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
            $entries = if ($userPath) { $userPath -split ';' } else { @() }
            if ($entries -notcontains $installDir) {
                $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
                [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
                if ($fancy) { Step 'ok' 'Adicionando ao PATH' 'reabra o terminal se necessario' }
                else { Write-Host "devzymer: $installDir adicionado ao PATH do usuario" }
            }
            elseif ($fancy) {
                Step 'ok' 'PATH' 'ja configurado'
            }
            if (($env:Path -split ';') -notcontains $installDir) {
                $env:Path = "$env:Path;$installDir"
            }
        }

        if ($fancy) {
            Out-Raw "`n"
            Out-Raw ("  " + (Paint (Gradient 0.5) ("$esc[1m" + 'Pronto. Proximo passo, dentro do seu projeto:')) + "`n`n")
            Out-Raw ('    ' + (Paint $white 'devzymer init') + (Paint $muted '   # escolha os agentes e indexe o projeto') + "`n`n")
        }
        else {
            Write-Host ''
            Write-Host 'Proximo passo, dentro do seu projeto:'
            Write-Host '  devzymer init'
        }
    }
    finally {
        if ($animate) { Out-Raw "$esc[?25h" }
        if ($null -ne $previousEncoding) { try { [Console]::OutputEncoding = $previousEncoding } catch { } }
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Devzymer
