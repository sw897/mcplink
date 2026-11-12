param([Parameter(Mandatory=$true)][int]$DesktopProcess,
      [Parameter(Mandatory=$true)][string]$DesktopPath)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class McplinkWindow {
  private delegate bool ChildCallback(IntPtr hwnd, IntPtr data);
  [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, ChildCallback callback, IntPtr data);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr hwnd, StringBuilder name, int count);
  [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd, int x, int y, int width, int height, bool repaint);
  public static string ClassName(IntPtr hwnd) {
    var name = new StringBuilder(256);
    GetClassName(hwnd, name, name.Capacity);
    return name.ToString();
  }
  public static IntPtr[] Candidates(IntPtr host, uint owner) {
    var windows = new List<IntPtr>();
    uint process;
    if (host == IntPtr.Zero || GetWindowThreadProcessId(host, out process) == 0 || process != owner) {
      return windows.ToArray();
    }
    windows.Add(host);
    EnumChildWindows(host, delegate(IntPtr child, IntPtr data) {
      windows.Add(child);
      return true;
    }, IntPtr.Zero);
    return windows.ToArray();
  }
}
'@
$script:handle = [IntPtr]::Zero
$script:providerHandle = [IntPtr]::Zero
function Window-State {
    $counts = @{}
    $providers = @()
    foreach ($hwnd in [McplinkWindow]::Candidates($script:handle, $DesktopProcess)) {
        $name = [McplinkWindow]::ClassName($hwnd)
        if ($name -notin @('Tauri Window', 'Tao Window', 'WRY_WEBVIEW', 'Chrome_WidgetWin_0', 'Chrome_WidgetWin_1', 'Chrome_RenderWidgetHostHWND', '#32770', 'Static', 'Button')) { $name = 'other' }
        if (-not $counts.ContainsKey($name)) { $counts[$name] = 0 }
        $counts[$name]++
        try {
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
            $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            $buttons = 0
            $entry = ''
            foreach ($node in $nodes) {
                if ($node.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button) { $buttons++ }
                if ($node.Current.Name -eq '统一入口') { $entry = $node.Current.ControlType.ProgrammaticName }
            }
            $providers += @{class=$name; elements=$nodes.Count; buttons=$buttons; entryType=$entry}
        } catch { $providers += @{class=$name; unavailable=$true} }
    }
    Write-Output ('NATIVE_HWND ' + ($counts | ConvertTo-Json -Compress))
    Write-Output ('NATIVE_PROVIDERS ' + (ConvertTo-Json -InputObject @($providers) -Compress -Depth 4))
}
function Content-Root {
    # wry's top-level HWND may not stitch its child WebView2 provider into UIA.
    # Scope the real native content HWND through the owned host's child chain.
    $candidates = [McplinkWindow]::Candidates($script:handle, $DesktopProcess)
    if ($script:providerHandle -ne [IntPtr]::Zero -and $script:providerHandle -in $candidates) {
        return [System.Windows.Automation.AutomationElement]::FromHandle($script:providerHandle)
    }
    $script:providerHandle = [IntPtr]::Zero
    foreach ($content in $candidates) {
        try {
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($content)
            $name = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, '统一入口')
            $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $name)
            if ($found -and $found.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button) {
                $script:providerHandle = $content
                return $root
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            # Read-only discovery can race a renderer's native HWND creation.
        } catch [System.Runtime.InteropServices.COMException] {
            # A host/container HWND may not supply an accessibility provider.
        }
    }
}
trap {
    Window-State
    Write-Output ('NATIVE_ERROR ' + $_.Exception.GetType().Name)
    exit 1
}
function Wait-Until([scriptblock]$Condition, [string]$Description) {
    $deadline = [DateTime]::UtcNow.AddSeconds(35)
    do {
        $value = & $Condition
        if ($value) { return $value }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Native desktop did not reach state: $Description"
}
function Find-Button([string]$Name) {
    $root = Content-Root
    if (-not $root) { return $null }
    $condition = New-Object System.Windows.Automation.AndCondition(
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Name)),
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)))
    return $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}
function Click-Button([string]$Name) {
    Write-Output ('NATIVE_ACTION ' + $Name)
    $element = Wait-Until {
        $button = Find-Button $Name
        if ($button -and $button.Current.IsEnabled -and -not $button.Current.IsOffscreen) { $button }
    } $Name
    $pattern = $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
}
function Wait-Text([string]$Text) {
    Write-Output ('NATIVE_EXPECT ' + $Text)
    Wait-Until {
        $root = Content-Root
        if (-not $root) { return $false }
        $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        $names = foreach ($node in $nodes) { $node.Current.Name }
        (($names -join ' ') -replace '\s+', ' ').Contains($Text)
    } $Text | Out-Null
}
Write-Output 'NATIVE_STEP window'
$script:handle = Wait-Until {
    $process = Get-Process -Id $DesktopProcess -ErrorAction Stop
    $process.Refresh()
    if ($process.MainWindowHandle -ne 0 -and [McplinkWindow]::IsWindowVisible($process.MainWindowHandle)) { $process.MainWindowHandle }
} 'visible owned Tauri window (interactive Windows desktop required)'
[McplinkWindow]::MoveWindow($script:handle, 0, 0, 1000, 740, $true) | Out-Null
[McplinkWindow]::SetForegroundWindow($script:handle) | Out-Null
Write-Output 'NATIVE_STEP provider'
Window-State
Wait-Until { Content-Root } 'owned WebView2 native accessibility provider' | Out-Null
Write-Output 'NATIVE_STEP navigation'
Click-Button '统一入口'
Wait-Until { Find-Button '创建统一入口' } 'entry navigation' | Out-Null
Click-Button '创建统一入口'
Wait-Until { Find-Button '保存入口' } 'native WebView modal' | Out-Null
Click-Button '取消'
Wait-Until { -not (Find-Button '保存入口') } 'modal closed' | Out-Null
Write-Output 'PASS native Tauri window, navigation and modal cancellation'
Write-Output 'NATIVE_STEP http-probe'
Click-Button 'MCP 服务'
Click-Button 'Windows API 验收 详情'
Click-Button '运行探测'
Wait-Text 'POST /echo/{id}'
Click-Button '取消'
Click-Button 'Windows API 验收 详情'
Click-Button '运行探测'
Wait-Text 'POST /echo/{id}'
Click-Button '确认并运行一次'
Wait-Text '真实探测调用已通过。'
Click-Button '关闭'
Write-Output 'PASS native HTTP API probe confirmation and actual fixture call'
Write-Output 'NATIVE_STEP background'
Click-Button '后台运行'
Click-Button '暂停共享'
Wait-Until { Find-Button '恢复共享' } 'pause persisted through native RPC' | Out-Null
Click-Button '恢复共享'
Wait-Until { Find-Button '暂停共享' } 'resume persisted through native RPC' | Out-Null
Write-Output 'PASS native pause and resume controls'
Write-Output 'NATIVE_STEP tray-reopen'
if (-not [McplinkWindow]::PostMessage($script:handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Cannot close owned window' }
Wait-Until { -not [McplinkWindow]::IsWindowVisible($script:handle) } 'window hidden to tray' | Out-Null
if ((Get-Process -Id $DesktopProcess).HasExited) { throw 'Closing the window exited the desktop process' }
$reopen = Start-Process -FilePath $DesktopPath -PassThru
try {
    Wait-Until { [McplinkWindow]::IsWindowVisible($script:handle) } 'single-instance reopen' | Out-Null
    if (-not $reopen.WaitForExit(15000) -or $reopen.ExitCode -ne 0) { throw 'Second desktop instance did not hand off cleanly' }
} finally {
    if (-not $reopen.HasExited) { $reopen.Kill() }
}
Wait-Until { Find-Button '暂停共享' } 'restored native window' | Out-Null
Write-Output 'PASS close-to-tray and single-instance window reopening'
