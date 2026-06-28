# Test IoTService IPC pipe - send query and read with manual timeout
$pipePath = "\\.\pipe\LOCAL\IoTService_IPC_Broker"

Write-Host "=== Testing IoTService IPC Pipe ==="

try {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "LOCAL\IoTService_IPC_Broker", [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::Asynchronous, [System.Security.Principal.TokenImpersonationLevel]::Impersonation)
    $pipe.Connect(5000)
    $pipe.ReadMode = [System.IO.Pipes.PipeTransmissionMode]::Message
    
    Write-Host "Connected to pipe successfully!"
    
    # Build MCPI header for GetModel (0x1001) - simplest query
    $header = New-Object byte[] 16
    $header[0] = 0x4D; $header[1] = 0x43; $header[2] = 0x50; $header[3] = 0x49
    $header[4] = 0x01; $header[5] = 0x00  # src_id = 1
    $header[6] = 0x02; $header[7] = 0x00  # dst_id = 2
    $header[8] = 0x01; $header[9] = 0x10  # type_lo = 0x1001 (GetModel)
    $header[10] = 0x00; $header[11] = 0x00  # routing = 0
    $header[12] = 0x00; $header[13] = 0x00  # field = 0
    $header[14] = 0x10; $header[15] = 0x00  # payload_len = 16
    
    Write-Host "Sending GetModel (0x1001) query..."
    
    $pipe.Write($header, 0, $header.Length)
    $pipe.Flush()
    
    Write-Host "Waiting for response (5s timeout)..."
    
    # Use async read with timeout
    $respBuf = New-Object byte[] 4096
    $asyncResult = $pipe.BeginRead($respBuf, 0, $respBuf.Length, $null, $null)
    $completed = $asyncResult.AsyncWaitHandle.WaitOne(5000)
    
    if ($completed) {
        $bytesRead = $pipe.EndRead($asyncResult)
        Write-Host "Response: $bytesRead bytes"
        Write-Host "Hex: $(($respBuf[0..([Math]::Min($bytesRead-1, 63))] | ForEach-Object { '{0:X2}' -f $_ }) -join ' ')"
        
        if ($bytesRead -ge 16) {
            $magic = [BitConverter]::ToUInt32($respBuf, 0)
            $srcId = [BitConverter]::ToUInt16($respBuf, 4)
            $dstId = [BitConverter]::ToUInt16($respBuf, 6)
            $typeLo = [BitConverter]::ToUInt16($respBuf, 8)
            $payloadLen = [BitConverter]::ToUInt16($respBuf, 14)
            $actualPayload = $payloadLen - 16
            
            Write-Host ""
            Write-Host "Response Header:"
            Write-Host "  Magic: 0x$($magic.ToString('X8'))"
            Write-Host "  SrcId: $srcId"
            Write-Host "  DstId: $dstId"
            Write-Host "  TypeLo: 0x$($typeLo.ToString('X4'))"
            Write-Host "  PayloadLen: $payloadLen (payload: $actualPayload bytes)"
            
            if ($actualPayload -gt 0 -and $bytesRead -ge $payloadLen) {
                $payloadBytes = $respBuf[16..($payloadLen - 1)]
                $payloadText = [System.Text.Encoding]::UTF8.GetString($payloadBytes)
                Write-Host "  Payload: $payloadText"
            }
        }
    } else {
        Write-Host "Timeout: No response within 5 seconds"
        Write-Host "This may mean:"
        Write-Host "  - IoTService doesn't respond to our client ID (1)"
        Write-Host "  - The message type requires a different format"
        Write-Host "  - The service is busy processing other requests"
    }
    
    $pipe.Close()
} catch {
    Write-Host "Error: $_"
}
