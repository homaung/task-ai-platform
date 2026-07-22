param([string]$Mode)

$ErrorActionPreference = 'Stop'
$line = [Console]::In.ReadLine()
if ([string]::IsNullOrWhiteSpace($line)) {
    exit 2
}

$request = $line | ConvertFrom-Json
$result = switch ($request.method) {
    'getPluginInfo' {
        @{
            id = 'dev.taskai.mock-provider'
            version = '1.0.0'
        }
    }
    'validateAccount' {
        @{
            valid = $true
            warnings = @()
        }
    }
    'discoverModels' {
        @{
            models = @(
                @{
                    providerModelKey = 'mock-general'
                    displayName = 'Mock General'
                    capabilities = @('chat', 'streaming', 'session_resume', 'structured_output')
                },
                @{
                    providerModelKey = 'mock-coder'
                    displayName = 'Mock Coder'
                    capabilities = @('chat', 'streaming', 'session_resume', 'filesystem_read', 'filesystem_write', 'command_execution', 'code_edit')
                }
            )
        }
    }
    'getCapabilities' {
        @{
            capabilities = @('chat', 'streaming', 'session_resume', 'structured_output', 'filesystem_read', 'filesystem_write', 'command_execution', 'code_edit', 'usage_reporting')
        }
    }
    'startSession' {
        $sessionId = [guid]::NewGuid().ToString()
        @{
            providerSessionReference = "mock-session-$sessionId"
            providerThreadReference = "mock-thread-$sessionId"
            metadata = @{
                deterministic = $true
                mode = $Mode
            }
            message = 'Mock provider session started.'
        }
    }
    'resumeSession' {
        @{
            resumed = $true
            providerSessionReference = $request.params.providerSessionReference
            metadata = @{ deterministic = $true }
        }
    }
    'sendMessage' {
        @{
            events = @(
                @{ type = 'message_delta'; text = 'Mock response' },
                @{ type = 'message_completed'; usage = @{ inputTokens = 1; outputTokens = 2 } }
            )
        }
    }
    'cancelSession' {
        @{ cancelled = $true }
    }
    'summarizeSession' {
        @{ summary = 'Deterministic mock session.' }
    }
    'getUsage' {
        @{ inputTokens = 1; outputTokens = 2; cost = 0 }
    }
    default {
        $null
    }
}

if ($null -eq $result) {
    $response = @{
        id = $request.id
        error = @{
            code = 'method_not_supported'
            message = "Unsupported method: $($request.method)"
        }
    }
} else {
    $response = @{
        id = $request.id
        result = $result
    }
}

[Console]::Out.WriteLine(($response | ConvertTo-Json -Compress -Depth 20))

