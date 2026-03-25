# End-to-end test for Copilot Rewrite translation pipeline
# Uses .NET HttpClient to avoid PowerShell header parsing issues with Copilot tokens

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# ── Load saved auth ──
$authPath = "$env:APPDATA\copilot-rewrite\auth.json"
if (-not (Test-Path $authPath)) {
    Write-Host "❌ No auth file found. Please login via the app first." -ForegroundColor Red
    exit 1
}
$auth = Get-Content $authPath -Raw | ConvertFrom-Json
$githubToken = $auth.github_token
Write-Host "✅ Loaded auth for user: $($auth.username)" -ForegroundColor Green
Write-Host ""

# ── Helper: HTTP request using .NET HttpClient (handles Copilot tokens with semicolons) ──
function Invoke-CopilotRequest {
    param(
        [string]$Uri,
        [string]$Method = "GET",
        [hashtable]$Headers,
        [string]$Body
    )
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(30)
    
    $request = [System.Net.Http.HttpRequestMessage]::new()
    $request.Method = [System.Net.Http.HttpMethod]::new($Method)
    $request.RequestUri = [Uri]::new($Uri)
    
    foreach ($key in $Headers.Keys) {
        if ($key -eq "Content-Type") { continue }
        $request.Headers.TryAddWithoutValidation($key, $Headers[$key]) | Out-Null
    }
    
    if ($Body) {
        $request.Content = [System.Net.Http.StringContent]::new($Body, [System.Text.Encoding]::UTF8, "application/json")
    }
    
    $response = $client.SendAsync($request).GetAwaiter().GetResult()
    $responseBody = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    
    $client.Dispose()
    
    return @{
        StatusCode = [int]$response.StatusCode
        Body       = $responseBody
    }
}

# ── Step 1: Exchange GitHub token for Copilot session token ──
Write-Host "━━━ Step 1: Get Copilot Session Token ━━━" -ForegroundColor Cyan
$tokenResp = Invoke-CopilotRequest -Uri "https://api.github.com/copilot_internal/v2/token" -Headers @{
    "Authorization" = "token $githubToken"
    "User-Agent"    = "CopilotRewrite/0.1.0"
    "Accept"        = "application/json"
}

if ($tokenResp.StatusCode -ne 200) {
    Write-Host "❌ Failed to get Copilot token (HTTP $($tokenResp.StatusCode))" -ForegroundColor Red
    Write-Host "   Response: $($tokenResp.Body)" -ForegroundColor Gray
    Write-Host ""
    Write-Host "   Possible causes:" -ForegroundColor Yellow
    Write-Host "   - GitHub token expired or invalid" -ForegroundColor Yellow
    Write-Host "   - No active Copilot subscription" -ForegroundColor Yellow
    exit 1
}

$tokenData = $tokenResp.Body | ConvertFrom-Json
$copilotToken = $tokenData.token
$expiresAt = [DateTimeOffset]::FromUnixTimeSeconds($tokenData.expires_at).LocalDateTime
Write-Host "✅ Got Copilot session token" -ForegroundColor Green
Write-Host "   Expires: $expiresAt"

# Check for proxy endpoint in token (but always use api.githubcopilot.com for chat)
$chatUrl = "https://api.githubcopilot.com/chat/completions"
if ($copilotToken -match "proxy-ep=([^;]+)") {
    $proxyEndpoint = $Matches[1]
    Write-Host "   Enterprise proxy: $proxyEndpoint"
}
Write-Host "   Chat URL: $chatUrl"
Write-Host ""

# ── Step 2: List available models ──
Write-Host "━━━ Step 2: List Available Models ━━━" -ForegroundColor Cyan
$modelsUrl = $chatUrl -replace "/chat/completions$", "/models"
$modelsResp = Invoke-CopilotRequest -Uri $modelsUrl -Headers @{
    "Authorization"           = "Bearer $copilotToken"
    "User-Agent"              = "CopilotRewrite/0.1.0"
    "Accept"                  = "application/json"
    "Editor-Version"          = "CopilotRewrite/0.1.0"
    "Editor-Plugin-Version"   = "CopilotRewrite/0.1.0"
    "Copilot-Integration-Id"  = "vscode-chat"
    "Openai-Intent"           = "model-access"
}

if ($modelsResp.StatusCode -eq 200) {
    $modelsData = $modelsResp.Body | ConvertFrom-Json
    if ($modelsData.data) {
        Write-Host "✅ Available models ($($modelsData.data.Count)):" -ForegroundColor Green
        foreach ($m in $modelsData.data) {
            $capabilities = @()
            if ($m.capabilities.type -eq "chat") { $capabilities += "chat" }
            Write-Host "   • $($m.id)$(if ($m.name -and $m.name -ne $m.id) { " ($($m.name))" })" -ForegroundColor White
        }
    }
} else {
    Write-Host "⚠️ Models endpoint returned $($modelsResp.StatusCode)" -ForegroundColor Yellow
    Write-Host "   $($modelsResp.Body.Substring(0, [Math]::Min(200, $modelsResp.Body.Length)))" -ForegroundColor Gray
}
Write-Host ""

# ── Step 3: Test Translation ──
Write-Host "━━━ Step 3: Run Translation Tests ━━━" -ForegroundColor Cyan

$testCases = @(
    @{
        Name   = "Chinese → English (Translate + Polish)"
        Input  = "今天天气真好，我想出去走走。"
        Action = "TranslateAndPolish"
        Lang   = "English"
    },
    @{
        Name   = "Technical Chinese → English"
        Input  = "我们需要优化这个API的性能，目前响应时间太长了，用户体验很差。建议使用缓存和异步处理来改善。"
        Action = "TranslateAndPolish"
        Lang   = "English"
    },
    @{
        Name   = "English → Polish Only"
        Input  = "i think we shuld probbaly fix this bug befor the release, its very importent for user experiance"
        Action = "Polish"
        Lang   = "English"
    },
    @{
        Name   = "English → Chinese"
        Input  = "The quick brown fox jumps over the lazy dog."
        Action = "Translate"
        Lang   = "Chinese (Simplified)"
    }
)

$model = "claude-sonnet-4"
$passed = 0
$failed = 0

foreach ($tc in $testCases) {
    Write-Host ""
    Write-Host "  ── Test: $($tc.Name) ──" -ForegroundColor White
    Write-Host "  Input:  $($tc.Input)" -ForegroundColor Gray

    switch ($tc.Action) {
        "Translate" {
            $sysPrompt = "You are a professional translator. Translate the given text into $($tc.Lang). Auto-detect the source language. Return ONLY the translated text, no explanations."
        }
        "Polish" {
            $sysPrompt = "You are a professional writing assistant. Polish and improve the given text. Fix grammar, spelling, punctuation. Improve clarity and readability. Keep the same language. Return ONLY the polished text."
        }
        "TranslateAndPolish" {
            $sysPrompt = "You are a professional translator and writing assistant. Translate the given text into $($tc.Lang) and polish it for clarity and fluency. Return ONLY the translated and polished text, no explanations."
        }
    }

    $body = @{
        model       = $model
        messages    = @(
            @{ role = "system"; content = $sysPrompt },
            @{ role = "user";   content = $tc.Input }
        )
        temperature = 0.3
        stream      = $true
    } | ConvertTo-Json -Depth 5

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    
    $resp = Invoke-CopilotRequest -Uri $chatUrl -Method "POST" -Headers @{
        "Authorization"           = "Bearer $copilotToken"
        "Content-Type"            = "application/json"
        "User-Agent"              = "CopilotRewrite/0.1.0"
        "Editor-Version"          = "CopilotRewrite/0.1.0"
        "Editor-Plugin-Version"   = "CopilotRewrite/0.1.0"
        "Copilot-Integration-Id"  = "vscode-chat"
        "Openai-Intent"           = "conversation-panel"
    } -Body $body
    
    $sw.Stop()
    $elapsed = $sw.ElapsedMilliseconds

    if ($resp.StatusCode -eq 200) {
        # Parse SSE stream response
        $result = ""
        $usedModel = $model
        foreach ($line in $resp.Body -split "`n") {
            $line = $line.Trim()
            if ($line -like "data: *" -and $line -ne "data: [DONE]") {
                $jsonStr = $line.Substring(6)
                try {
                    $chunk = $jsonStr | ConvertFrom-Json
                    if ($chunk.choices -and $chunk.choices[0].delta.content) {
                        $result += $chunk.choices[0].delta.content
                    }
                    if ($chunk.model) { $usedModel = $chunk.model }
                } catch { }
            }
        }
        $result = $result.Trim()
        
        Write-Host "  Output: $result" -ForegroundColor Green
        Write-Host "  Model: $usedModel  |  Time: ${elapsed}ms" -ForegroundColor DarkGray
        
        if ([string]::IsNullOrWhiteSpace($result)) {
            Write-Host "  ❌ FAIL: Empty result" -ForegroundColor Red
            $failed++
        } elseif ($result -eq $tc.Input) {
            Write-Host "  ⚠️ WARN: Output identical to input" -ForegroundColor Yellow
            $passed++
        } else {
            Write-Host "  ✅ PASS" -ForegroundColor Green
            $passed++
        }
    } else {
        Write-Host "  ❌ FAIL (HTTP $($resp.StatusCode)): $($resp.Body.Substring(0, [Math]::Min(200, $resp.Body.Length)))" -ForegroundColor Red
        $failed++
    }
}

Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
Write-Host "  Results: $passed passed, $failed failed (out of $($testCases.Count))" -ForegroundColor $(if ($failed -eq 0) { "Green" } else { "Red" })
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan
