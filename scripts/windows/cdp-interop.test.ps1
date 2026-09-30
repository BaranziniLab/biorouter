$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tokens = $null
$errors = $null
$source = Join-Path $PSScriptRoot 'normal-release-smoke.ps1'
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count -gt 0) { throw 'The normal acceptance helper did not parse' }
$functions = @($ast.FindAll({ param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-CDP'
}, $true))
if ($functions.Count -ne 1) { throw 'Expected the one real Invoke-CDP implementation' }
Invoke-Expression $functions[0].Extent.Text
Add-Type -TypeDefinition @'
using System;
using System.Net;
using System.Net.WebSockets;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
public static class ReleaseCDPLoopback {
    public static async Task Serve(HttpListener listener, string[] frames) {
        try {
            var context = await listener.GetContextAsync();
            using var websocket = (await context.AcceptWebSocketAsync(null)).WebSocket;
            var buffer = new byte[65536];
            var received = await websocket.ReceiveAsync(new ArraySegment<byte>(buffer), CancellationToken.None);
            if (!received.EndOfMessage || received.MessageType != WebSocketMessageType.Text)
                throw new Exception("Expected one synthetic CDP request frame");
            using var request = JsonDocument.Parse(Encoding.UTF8.GetString(buffer, 0, received.Count));
            if (request.RootElement.GetProperty("id").GetInt32() != 1 ||
                request.RootElement.GetProperty("method").GetString() != "Runtime.evaluate")
                throw new Exception("Synthetic CDP request identity did not match");
            foreach (var frame in frames) {
                var bytes = Encoding.UTF8.GetBytes(frame);
                await websocket.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, CancellationToken.None);
            }
        } finally { listener.Stop(); }
    }
}
'@
$success = '{"id":1,"result":{"result":{"type":"object","value":{"ready":true,"marker":"synthetic"}}}}'
$wrongId = '{"id":999,"result":{"result":{"type":"object","value":{"ready":false,"marker":"wrong"}}}}'
$errorFrame = '{"id":1,"error":{"code":-32602,"message":"synthetic failure"}}'
$passed = 0
foreach ($case in @('single-envelope', 'ignore-wrong-id', 'reject-error')) {
    $reservation = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $reservation.Start()
    $port = $reservation.LocalEndpoint.Port
    $reservation.Stop()
    $listener = [Net.HttpListener]::new()
    $listener.Prefixes.Add("http://127.0.0.1:$port/")
    $listener.Start()
    $frames = switch ($case) {
        'single-envelope' { @($success) }
        'ignore-wrong-id' { @($wrongId, $success) }
        'reject-error' { @($errorFrame) }
    }
    $serving = [ReleaseCDPLoopback]::Serve($listener, [string[]]$frames)
    try {
        if ($case -eq 'reject-error') {
            $rejected = $false
            try { $unexpected = Invoke-CDP -Socket "ws://127.0.0.1:$port/" -Method 'Runtime.evaluate' -Parameters @{ expression = 'true' } }
            catch { $rejected = $_.Exception.Message -eq 'Renderer inspector rejected Runtime.evaluate' }
            if (-not $rejected) { throw 'The exact CDP error response was not rejected' }
        } else {
            $envelope = Invoke-CDP -Socket "ws://127.0.0.1:$port/" -Method 'Runtime.evaluate' -Parameters @{ expression = 'true' }
            if ($envelope -is [array] -or $envelope -isnot [pscustomobject] -or
                $envelope.result.type -ne 'object' -or $envelope.result.value.ready -ne $true -or
                $envelope.result.value.marker -ne 'synthetic') { throw 'CDP did not return one exact method envelope/RemoteObject/value' }
            Write-Host ("$case completion classes: " + $script:lastCDPResponseShape.connectCompletionClass + ', ' +
                $script:lastCDPResponseShape.sendCompletionClass)
        }
        $serving.GetAwaiter().GetResult() | Out-Null
        $passed++
        Write-Host "PASS $case"
    } finally {
        $listener.Stop()
        $listener.Close()
    }
}
if ($passed -ne 3) { throw 'Not all native CDP interop assertions executed' }
Write-Host 'Native CDP interop: 3/3 passed'
