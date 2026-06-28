# Test IoTService IPC pipe directly from PowerShell
# This sends a GetDeviceStatus (0x1006) query and reads the response

$pipePath = "\\.\pipe\LOCAL\IoTService_IPC_Broker"

Write-Host "=== Testing IoTService IPC Pipe ==="
Write-Host "Pipe: $pipePath"

try {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "LOCAL\IoTService_IPC_Broker", [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::None, [System.Security.Principal.TokenImpersonationLevel]::Impersonation)
    $pipe.Connect(5000)
    $pipe.ReadMode = [System.IO.Pipes.PipeTransmissionMode]::Message
    
    Write-Host "Connected to pipe successfully!"
    
    # Build MCPI header for GetDeviceStatus (0x1006)
    # Header: magic(4) + src_id(2) + dst_id(2) + type_lo(2) + routing(2) + field(2) + payload_len(2) = 16 bytes
    # magic = 0x4950434D ("MCPI" in LE)
    # src_id = 1 (us), dst_id = 2 (IoTDriver)
    # type_lo = 0x1006 (GetDeviceStatus)
    # routing = 0, field = 0
    # payload_len = 16 (header only, no payload)
    
    $header = New-Object byte[] 16
    # Magic: 4D 43 50 49 (MCPI in LE)
    $header[0] = 0x4D; $header[1] = 0x43; $header[2] = 0x50; $header[3] = 0x49
    # src_id = 1
    $header[4] = 0x01; $header[5] = 0x00
    # dst_id = 2
    $header[6] = 0x02; $header[7] = 0x00
    # type_lo = 0x1006
    $header[8] = 0x06; $header[9] = 0x10
    # routing = 0
    $header[10] = 0x00; $header[11] = 0x00
    # field = 0
    $header[12] = 0x00; $header[13] = 0x00
    # payload_len = 16 (total message size = header only)
    $header[14] = 0x10; $header[15] = 0x00
    
    Write-Host "Sending GetDeviceStatus (0x1006) query..."
    Write-Host "Header bytes: $($header | ForEach-Object { '{0:X2}' -f $_ })"
    
    $pipe.Write($header, 0, $header.Length)
    $pipe.Flush()
    
    # Read response
    $respBuf = New-Object byte[] 4096
    $bytesRead = $pipe.Read($respBuf, 0, $respBuf.Length)
    
    Write-Host "Response: $bytesRead bytes"
    Write-Host "Hex: $(($respBuf[0..([Math]::Min($bytesRead-1, 63))] | ForEach-Object { '{0:X2}' -f $_ }) -join ' ')"
    
    if ($bytesRead -ge 16) {
        # Parse response header
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
    
    $pipe.Close()
} catch {
    Write-Host "Error: $_"
}
