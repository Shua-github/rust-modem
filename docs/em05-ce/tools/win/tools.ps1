# Windows 侧的两个小工具：
#
#   at-com       用串口（模组的 AT 口，形如 COMx）发 AT
#     powershell -File tools.ps1 at-com -Port COMx -Commands 'ATI'
#
#   reenumerate  禁用 + 启用指定设备实例，尝试软重枚举（驱动切换挂起时仍需物理拔插/重启）
#     实例号可用 Get-PnpDevice 查，例如：
#       Get-PnpDevice -PresentOnly | Where-Object InstanceId -like '*VID_2C7C&PID_0127*'

param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('at-com', 'reenumerate')]
    [string] $Action,
    [string] $Port = 'COM1',
    [string[]] $Commands = @(),
    [string] $InstanceId = '',
    [int] $Baud = 115200,
    [int] $WaitMs = 1200
)

switch ($Action) {
    'at-com' {
        $serial = New-Object System.IO.Ports.SerialPort $Port, $Baud
        $serial.NewLine = "`r"
        $serial.ReadTimeout = $WaitMs
        $serial.WriteTimeout = 2000
        $serial.DtrEnable = $true
        $serial.RtsEnable = $true
        $serial.Open()
        try {
            foreach ($command in $Commands) {
                $serial.WriteLine($command)
                Start-Sleep -Milliseconds $WaitMs
                $answer = ''
                try { $answer = $serial.ReadExisting() } catch { $answer = '' }
                Write-Output ">>> $command"
                Write-Output ($answer.Trim())
            }
        } finally {
            $serial.Close()
        }
    }
    'reenumerate' {
        if ([string]::IsNullOrWhiteSpace($InstanceId)) {
            throw 'reenumerate 需要 -InstanceId'
        }
        Write-Output "disable $InstanceId"
        Disable-PnpDevice -InstanceId $InstanceId -Confirm:$false -ErrorAction Stop
        Start-Sleep -Seconds 5
        Write-Output "enable  $InstanceId"
        Enable-PnpDevice -InstanceId $InstanceId -Confirm:$false -ErrorAction Stop
        Write-Output 're-enumerated'
    }
}
