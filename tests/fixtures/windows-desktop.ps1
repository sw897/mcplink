param([Parameter(Mandatory=$true)][int]$DesktopProcess,
      [Parameter(Mandatory=$true)][string]$DesktopPath)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class McplinkWindow {
  private delegate bool ChildCallback(IntPtr hwnd, IntPtr data);
  [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, ChildCallback callback, IntPtr data);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr hwnd, StringBuilder name, int count);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hwnd, int x, int y, int width, int height, bool repaint);
  public static IntPtr WebContent(IntPtr host) {
    IntPtr content = IntPtr.Zero;
    EnumChildWindows(host, delegate(IntPtr child, IntPtr data) {
      var name = new StringBuilder(256);
      GetClassName(child, name, name.Capacity);
      if (name.ToString() == "Chrome_RenderWidgetHostHWND" && IsWindowVisible(child)) {
        content = child;
        return false;
      }
      return true;
    }, IntPtr.Zero);
    return content;
  }
}
'@
$script:handle = [IntPtr]::Zero
function Content-Root {
    # wry's top-level HWND may not stitch its child WebView2 provider into UIA.
    # Scope the real native content HWND through the owned host's child chain.
    $content = [McplinkWindow]::WebContent($script:handle)
    if ($content -ne [IntPtr]::Zero) {
        return [System.Windows.Automation.AutomationElement]::FromHandle($content)
    }
}
trap {
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
