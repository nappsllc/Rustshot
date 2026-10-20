# Run inside the MSIX's package (v066.ps1 starts it with
# Invoke-CommandInDesktopPackage): reads the package's startup tasks with
# the StartupTask API, which needs the package identity, and with -Mode
# disable / enable first turns the "RustshotDaemon" task off (Disable(),
# what an app's own switch does) or on again (RequestEnableAsync()).
# Writes "task <TaskId> <State>" lines (or "error <message>") and END to -Out.
param([string]$Mode = 'list', [Parameter(Mandatory)] [string]$Out)
$ErrorActionPreference = 'Stop'
$lines = @()
try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $null = [Windows.ApplicationModel.StartupTask, Windows.ApplicationModel, ContentType = WindowsRuntime]
    $asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
    } | Select-Object -First 1
    function Await($op, [type]$type) {
        $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op))
        try { if (-not $t.Wait(30000)) { throw 'timed out' } } catch { throw $_.Exception.GetBaseException() }
        $t.Result
    }
    $daemon = Await ([Windows.ApplicationModel.StartupTask]::GetAsync('RustshotDaemon')) ([Windows.ApplicationModel.StartupTask])
    if ($Mode -eq 'disable') { $daemon.Disable() }
    if ($Mode -eq 'enable') {
        $s = Await ($daemon.RequestEnableAsync()) ([Windows.ApplicationModel.StartupTaskState])
        $lines += "enable -> $s"
    }
    $all = Await ([Windows.ApplicationModel.StartupTask]::GetForCurrentPackageAsync()) ([System.Collections.Generic.IReadOnlyList[Windows.ApplicationModel.StartupTask]])
    foreach ($x in $all) { $lines += "task $($x.TaskId) $($x.State)" }
} catch {
    $lines += "error $($_.Exception.Message)"
}
$lines += 'END'
Set-Content -Path $Out -Value $lines
