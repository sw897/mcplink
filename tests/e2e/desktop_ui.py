"""Exercise the built renderer against an isolated CLI, using a browser in place of the Tauri webview."""
import argparse
import functools
import http.server
import json
from pathlib import Path
import secrets
import subprocess
import threading
import time
import tomllib

from run import BINARY, Harness, ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--profile")
    parser.add_argument("--chrome", required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True)
    harness = Harness(record)
    secret = secrets.token_urlsafe(24)
    url = harness.url + "?tenant=sample&key=fixture"
    extra = harness.home / "second.toml"
    extra.write_text("# fixture source\n")
    result = {"conclusion":"FAIL"}
    checks = []

    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_POST(self):
            expected = f"127.0.0.1:{self.server.server_port}"
            if (self.path != "/rpc" or self.headers.get("Host") != expected
                    or self.headers.get("Origin") != "http://" + expected
                    or self.headers.get("X-Fixture") != secret):
                return self.send_error(403)
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if request.get("method") == "gateway.save" and request.get("params", {}).get("title") == "异步关闭入口":
                time.sleep(1.5)
            if request.get("method") not in {"snapshot","connections.detail","connections.save","connections.export_config","changes.preview","changes.apply","sources.add","connections.source_detail","connections.import","connections.detach","connections.probe","changes.preview_restore","changes.restore","credentials.disconnect","changes.history","gateway.save","connections.tools","tools.describe","tools.call","cli.generate","cli.inspect","cli.check","gateway.refresh"}:
                return self.send_error(403)
            if request["method"] == "sources.add" and request.get("params") != {"agent":"codex","path":str(extra)}:
                return self.send_error(403)
            proc = subprocess.run([str(BINARY),"rpc","--stdin"], input=json.dumps(request),
                                  text=True, capture_output=True, timeout=30, env=harness.environment)
            self.send_response(200)
            self.send_header("Content-Type","application/json")
            self.end_headers()
            self.wfile.write(proc.stdout.encode())

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1",0), functools.partial(Handler,directory=str(ROOT/"dist")))
    origin = f"http://127.0.0.1:{server.server_port}"
    threading.Thread(target=server.serve_forever, daemon=True).start()
    shim = record / "fixture.js"
    shim.write_text("""
if (location.origin === ORIGIN) {
  let sequence = 0;
  // Observe the clipboard adapter without overwriting the developer's OS clipboard.
  Object.defineProperty(navigator, "clipboard", {value:{async writeText(value) {window.__fixtureClipboard = value;}}});
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
  window.__TAURI_INTERNALS__ = {
    transformCallback() { return ++sequence; },
    async invoke(cmd, args) {
      if (cmd === "rpc") return (await fetch("/rpc", {
        method:"POST", headers:{"Content-Type":"application/json","X-Fixture":KEY},body:JSON.stringify(args)
      })).json();
      if (cmd.startsWith("plugin:event|")) return ++sequence;
      if (cmd === "plugin:dialog|open") {
        if (args?.options?.directory) return CLI_PARENT;
        if (args?.options?.filters?.[0]?.name === "mcplink CLI") return CLI_MANIFEST;
        return SOURCE_PATH;
      }
      if (cmd === "desktop_status") return {supported:false};
      throw Error("Unexpected desktop operation");
    }
  };
}
""".replace("ORIGIN",json.dumps(origin)).replace("KEY",json.dumps(secret)).replace("SOURCE_PATH", json.dumps(str(extra))).replace("CLI_PARENT",json.dumps(str(record))).replace("CLI_MANIFEST",json.dumps(str(record/"desktop-tools/desktop-tools.mcplink.json"))))

    def browser(*command):
        return subprocess.check_output(["agent-browser","--session",args.session,*command],
                                       text=True,timeout=55).strip()

    def audit(name):
        report = json.loads(browser("a11y","--json"))["data"]
        (record/f"a11y-{name}.json").write_text(json.dumps(report,indent=2))
        assert not report["violations"], [(v["id"],v["nodeCount"]) for v in report["violations"]]

    def wait_for(expression):
        browser("wait","--fn",expression)

    try:
        harness.rpc("connections.save", {"name":"public_http","title":"免认证服务","definition":{"url":url}})
        browser(*(["--profile",args.profile] if args.profile else []),
                "--executable-path",args.chrome,"--init-script",str(shim),"open",origin)
        browser("set","viewport","940","620")
        browser("wait","--text","免认证服务")
        audit("services")
        browser("find","role","button","click","--name","账号","--exact")
        wait_for("Array.from(document.querySelectorAll('article')).some(a=>a.textContent.includes('免认证服务') && a.textContent.includes('本机未配置独立凭据') && !a.textContent.includes('断开'))")
        checks.append("an unverified HTTP service shows the local credential state")
        browser("find","role","button","click","--name","Agent","--exact")
        browser("find","role","button","click","--name","添加来源","--exact")
        wait_for("document.querySelector('.agent-metrics strong')?.textContent === '2'")
        browser("find","first",".agent-services .cell-add","click")
        if "纳入托管并继续" in browser("snapshot"):
            browser("find","role","button","click","--name","纳入托管并继续")
        wait_for("document.querySelector('dialog[open]') && !document.querySelector('dialog input[type=checkbox]:checked')")
        browser("press","Escape")
        checks.append("Agent page adds a second Codex source and requires a target choice")
        browser("click","nav button:first-child")
        browser("find","role","button","click","--name","免认证服务 详情")
        browser("find","role","button","click","--name","编辑","--exact")
        browser("wait","--fn","document.querySelector('dialog input[required]') && !document.querySelector('dialog button.primary').disabled")
        browser("find","label","显示名称","fill","更名后的服务")
        browser("find","role","button","click","--name","保存连接")
        browser("wait","--text","更名后的服务")
        browser("click","article:first-of-type .service-card-footer > button")
        wait_for("document.querySelector('dialog[open]')?.contains(document.activeElement)")
        browser("press","Tab")
        wait_for("document.querySelector('dialog').contains(document.activeElement)")
        wait_for("document.querySelector('dialog footer').getBoundingClientRect().bottom <= innerHeight")
        audit("connect")
        browser("screenshot",str(record/"connect-min.png"))
        checks.append("minimum-size dialog keeps focus and preview button visible")
        wait_for("document.querySelector('dialog input[name=mode]:checked')?.value !== 'direct' && document.querySelector('dialog .mode-choices label.chosen')?.innerText.includes('单服务入口')")
        browser("click","dialog .mode-choices label:first-child")
        browser("find","first","dialog label.target input","click")
        browser("find","role","button","click","--name","预览变更")
        browser("wait","--text","核对配置变更")
        browser("wait","--text","明文")
        browser("find","role","button","click","--name","应用配置")
        browser("wait","--text","接入结果")
        assert tomllib.loads((harness.home/".codex/config.toml").read_text())["mcp_servers"]["public_http"]["url"] == url
        checks.append("rename, direct-write disclosure, preview and apply preserve exact URL in native configuration")
        def button(name):
            browser("find","role","button","click","--name",name,"--exact")

        button("完成")
        readonly = harness.home / "review-readonly.json"
        readonly.write_text(json.dumps({"mcpServers":{"readonly_notes":{"url":harness.url}}}))
        harness.rpc("sources.add",{"agent":"claude","path":str(readonly)})
        shared = harness.rpc("connections.save",{"name":"managed_docs","title":"研发文档","definition":{
            "url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"}}})["id"]
        harness.rpc("connections.probe",{"id":shared})
        harness.changes(shared,["hermes:user"],mode="shared")
        unauth = harness.rpc("connections.save",{"name":"team_search","title":"团队检索","definition":{"url":harness.url}})["id"]
        harness.rpc("connections.probe",{"id":unauth})
        button("刷新本机配置")
        browser("wait","--text","readonly_notes")
        browser("set","viewport","1180","780")
        wait_for("[...document.querySelectorAll('article.service-card')].filter(a=>a.getBoundingClientRect().bottom<=innerHeight).length >= 5")
        browser("screenshot",str(record/"compact-list.png"))
        button("紧凑列表")
        wait_for("!document.querySelector('.service-list.compact')")
        button("紧凑列表")
        button("readonly_notes 详情")
        button("编辑并纳入托管")
        wait_for("!document.querySelector('dialog button.primary').disabled")
        button("取消")
        assert next(c for c in harness.rpc("snapshot")["connections"] if c["name"]=="readonly_notes")["detected"]
        button("readonly_notes 详情")
        button("编辑并纳入托管")
        wait_for("!document.querySelector('dialog button.primary').disabled")
        button("保存并纳入托管")
        wait_for("!document.querySelector('dialog[open]')")
        button("readonly_notes 详情")
        button("解除绑定")
        button("确认")
        wait_for("!document.querySelector('dialog[open]')")
        assert json.loads(readonly.read_text())["mcpServers"]["readonly_notes"]["url"]==harness.url
        assert any(c.get("detected") and c["name"]=="readonly_notes" for c in harness.rpc("snapshot")["connections"])
        checks.append("compact layout, explicit read-only adoption, cancel and detach preserving source")

        button("研发文档 详情")
        button("管理")
        wait_for("document.querySelector('dialog .target input:checked')?.closest('label').innerText.includes('Hermes') && document.querySelector('dialog input[name=mode]:checked')?.parentElement.innerText.includes('单服务入口')")
        browser("screenshot",str(record/"manage-binding.png"))
        browser("press","Escape")
        button("研发文档 详情")
        button("编辑")
        wait_for("document.querySelector('dialog button.primary') && !document.querySelector('dialog button.primary').disabled")
        browser("find","label","MCP 地址","fill","http://127.0.0.1:1/mcp")
        wait_for("document.querySelector('dialog .warning-banner')?.innerText.includes('Hermes')")
        button("取消")
        button("研发文档 详情")
        button("检查工具")
        browser("wait","--text","search_documents")
        browser("press","Escape")
        button("账号")
        wait_for("[...document.querySelectorAll('article')].some(a=>a.innerText.includes('团队检索') && a.innerText.includes('需要认证') && a.innerText.includes('本机未配置独立凭据'))")
        button("断开")
        button("确认")
        wait_for("!document.querySelector('dialog[open]')")
        browser("click","nav button:first-child")
        button("研发文档 详情")
        wait_for("document.querySelector('dialog')?.innerText.includes('已断开') && !document.querySelector('dialog details.tool')")
        browser("press","Escape")
        checks.append("binding keeps Hermes/shared, address warning lists affected agents, disconnect removes stale tools")

        button("变更与恢复")
        browser("wait","--text","新增")
        browser("find","first",".history-row button","click")
        browser("wait","--text","核对恢复内容")
        browser("wait","--text","移除当前条目")
        browser("screenshot",str(record/"restore-preview.png"))
        button("返回记录")
        assert any(b.get("connectionId")==shared for b in harness.rpc("snapshot")["bindings"])
        browser("find","first",".history-row button","click")
        browser("wait","--text","核对恢复内容")
        button("确认恢复")
        wait_for("document.querySelector('dialog[aria-label=变更与恢复]') && !document.querySelector('dialog[aria-label=核对恢复内容]')")
        assert not any(b.get("connectionId")==shared for b in harness.rpc("snapshot")["bindings"])
        browser("press","Escape")
        checks.append("restore requires a preview, cancelling keeps the entry, confirming removes it")

        rotating = harness.rpc("connections.save",{"name":"rotating","title":"密钥轮换","definition":{
            "url":harness.url,"headers":{"Authorization":"synthetic-before"}}})["id"]
        harness.changes(rotating,["codex:user"],mode="direct")
        harness.rpc("credentials.update",{"id":rotating,"value":"synthetic-after"})
        button("刷新本机配置")
        browser("wait","--text","密钥轮换")
        button("密钥轮换 详情")
        button("管理")
        button("预览变更")
        wait_for("document.querySelector('.field-changes')?.innerText.includes('Authorization · 已更换')")
        assert "synthetic-before" not in browser("snapshot") and "synthetic-after" not in browser("snapshot")
        browser("screenshot",str(record/"credential-diff.png"))
        checks.append("secret rotation is visible in the field diff without either credential value")
        browser("press","Escape")

        # New office clients: prepare imports while uninstalled, without claiming native loading.
        harness.launchers["workbuddy"].unlink()
        harness.launchers["qwenwork"].unlink()
        harness.launchers["pi"].unlink()
        office_url = harness.url + "?ui-office=1"
        office_id = harness.rpc("connections.save", {"name":"office_ui","title":"办公接入验收","definition":{
            "url":office_url,"headers":{"Authorization":"synthetic-fixture-authorization"}}})["id"]
        harness.rpc("connections.probe", {"id":office_id})
        button("刷新本机配置")
        button("Agent")
        wait_for("!!document.querySelector('button[aria-label=\"查看 WorkBuddy（未安装）\"]')")
        button("查看 WorkBuddy（未安装）")
        wait_for("document.querySelector('.agent-unavailable')?.innerText.includes('未安装，暂不可配置') && document.querySelector('.agent-panel-actions button')?.disabled")
        checks.append("uninstalled WorkBuddy remains visible with native writes disabled")
        button("查看 Pi Agent（未安装）")
        wait_for("document.querySelector('.agent-unavailable')?.innerText.includes('未安装，暂不可配置') && document.querySelector('.agent-panel-actions button')?.disabled")
        checks.append("uninstalled Pi stays visible with native writes disabled and a CLI selection entry")

        before = harness.rpc("snapshot")
        browser("set","viewport","940","620")
        for client in ("千问办公", "豆包工作"):
            button(f"查看 {client}（未安装）")
            wait_for("document.querySelector('.agent-unavailable')?.innerText.includes('导入结果需在客户端确认') && !document.querySelector('.agent-source-details')")
            button(f"生成 办公接入验收 的 {client} 配置")
            wait_for("document.querySelectorAll('dialog .target input:checked').length === 1 && document.querySelector('dialog .target input:checked')?.closest('label').innerText.includes(" + json.dumps(client) + ")")
            browser("click","dialog .mode-choices label:first-child")
            button("生成导入配置")
            browser("wait","--text",f"{client} · 待客户端导入")
            wait_for("document.querySelector('dialog .modal-body').scrollTop === 0")
            browser("screenshot", str(record / ("qwen-import-top.png" if client == "千问办公" else "doubao-import-top.png")))
            wait_for("document.querySelector('dialog footer').getBoundingClientRect().bottom <= innerHeight && ![...document.querySelectorAll('dialog button')].some(b=>b.textContent==='应用配置')")
            if client == "千问办公":
                generated = json.loads(browser("eval", "document.querySelector('.import-config textarea').value"))
                assert json.loads(generated)["mcpServers"]["office_ui"]["url"] == office_url
                button("复制JSON 配置")
                wait_for("window.__fixtureClipboard === document.querySelector('.import-config textarea').value")
            else:
                assert json.loads(browser("eval", "document.querySelector('input[aria-label=\"服务器 URL\"]').value")) == office_url
                button("复制服务器 URL")
                wait_for("window.__fixtureClipboard === " + json.dumps(office_url))
                button("复制Header Authorization")
                wait_for("window.__fixtureClipboard === 'synthetic-fixture-authorization'")
            audit("qwen-import" if client == "千问办公" else "doubao-import")
            browser("screenshot", str(record / ("qwen-import.png" if client == "千问办公" else "doubao-import.png")))
            button("返回调整")
            browser("click","dialog .mode-choices label:nth-child(2)")
            button("生成导入配置")
            browser("wait","--text",f"{client} · 待客户端导入")
            if client == "千问办公":
                generated = json.loads(browser("eval", "document.querySelector('.import-config textarea').value"))
                definition = json.loads(generated)["mcpServers"]["office_ui"]
                assert definition["command"] == str(BINARY) and definition["args"] == ["bridge","--connection",office_id]
            else:
                assert json.loads(browser("eval", "document.querySelector('input[aria-label=运行命令]').value")) == str(BINARY)
                button("复制参数 3")
                wait_for("window.__fixtureClipboard === " + json.dumps(office_id))
            assert "synthetic-fixture-authorization" not in browser("snapshot")
            button("完成")
        after = harness.rpc("snapshot")
        assert before["bindings"] == after["bindings"] and before["history"] == after["history"]
        assert not any(b["agent"] in {"qwenwork","doubaowork"} for b in after["bindings"])
        checks.append("QwenWork JSON and DoubaoWork form values reach the clipboard adapter in direct and shared modes, with no binding or history mutation")

        harness.install_clients()
        button("刷新本机配置")
        wait_for("!!document.querySelector('button[aria-label=\"查看 千问办公\"]')")
        button("查看 千问办公")
        wait_for("!!document.querySelector('.agent-metrics') && !!document.querySelector('.agent-source-details')")
        if json.loads(browser("eval", "!!document.querySelector('.toast')")):
            button("关闭提示")
        browser("click", ".agent-source-details summary")
        wait_for("document.querySelector('.agent-source-details')?.innerText.includes('.qwenworkcn')")
        row = json.loads(browser("eval", "[...document.querySelectorAll('.agent-service-row')].findIndex(row=>row.querySelector('.service-name')?.textContent.includes('办公接入验收')) + 1"))
        assert row > 0
        browser("click", f".agent-service-row:nth-child({row}) .cell-add")
        wait_for("document.querySelectorAll('dialog .target input:checked').length === 1 && !document.querySelector('dialog .target input:checked').closest('label').innerText.includes('客户端导入')")
        browser("click", "dialog .mode-choices label:first-child")
        button("预览变更")
        browser("wait", "--text", "核对配置变更")
        button("应用配置")
        browser("wait", "--text", "接入结果")
        qwen_path = harness.home / ".qwenworkcn/mcp.json"
        assert json.loads(qwen_path.read_text())["mcpServers"]["office_ui"]["url"] == office_url
        button("完成")
        audit("qwen-native")
        browser("screenshot", str(record/"qwen-native.png"))
        checks.append("installed QwenWork selects its native source by default and applies the exact HTTP entry through preview")

        import sys, os
        qwen_profile = harness.home / ("Library/Application Support/QwenWorkCN" if sys.platform=="darwin" else "AppData/Roaming/QwenWorkCN" if os.name=="nt" else ".config/QwenWorkCN")
        marker = qwen_profile / "data/agents.identity-v2.db"
        marker.parent.mkdir(parents=True, exist_ok=True);marker.touch()
        before_private = qwen_path.read_bytes()
        button("刷新本机配置")
        wait_for("document.querySelector('.agent-unavailable')?.innerText.includes('身份隔离') && !document.querySelector('.agent-source-details')")
        button("生成 办公接入验收 的 千问办公 配置")
        button("生成导入配置")
        browser("wait", "--text", "千问办公 · 待客户端导入")
        assert qwen_path.read_bytes() == before_private
        button("完成")
        audit("qwen-private-fallback")
        browser("screenshot", str(record/"qwen-private-fallback.png"))
        marker.unlink()
        checks.append("QwenWork identity storage switches the UI to import with a concrete reason and leaves legacy JSON untouched")

        button("查看 Pi Agent")
        wait_for("!!document.querySelector('.agent-metrics') && !!document.querySelector('.agent-source-details')")
        row = json.loads(browser("eval", "[...document.querySelectorAll('.agent-service-row')].findIndex(row=>row.querySelector('.service-name')?.textContent.includes('办公接入验收')) + 1"))
        assert row > 0
        browser("click", f".agent-service-row:nth-child({row}) .cell-add")
        wait_for("document.querySelectorAll('dialog .target input:checked').length === 1 && !document.querySelector('dialog .target input:checked').closest('label').innerText.includes('客户端导入')")
        browser("click", "dialog .mode-choices label:first-child")
        button("预览变更")
        browser("wait", "--text", "核对配置变更")
        button("应用配置")
        browser("wait", "--text", "接入结果")
        entry = json.loads((harness.home / '.pi/agent/mcp.json').read_text())["mcpServers"]["office_ui"]
        assert entry["url"] == office_url and entry["type"] == "http" and "transport" not in entry
        button("完成")
        audit("pi-native")
        browser("screenshot", str(record/"pi-native.png"))
        browser("click", ".agent-panel-actions button")
        wait_for("document.querySelector('dialog')?.innerText.includes('Pi 1.x') && !!document.querySelector('dialog option[value=project]')")
        assert json.loads(browser("eval", "!document.querySelector('dialog input[placeholder=\"完整 JSON 文件路径\"]')"))
        button("关闭")
        checks.append("installed Pi applies strict native HTTP through preview and explains its built-in MCP and project trust requirements")
        wait_for("!!document.querySelector('dialog[aria-label=配置来源]')")
        button("关闭")
        wait_for("!document.querySelector('dialog[open]')")

        browser("click", "nav button:first-child")
        button("办公接入验收 详情")
        button("编辑")
        wait_for("document.querySelector('dialog .modal-body')?.scrollHeight > document.querySelector('dialog .modal-body')?.clientHeight")
        browser("eval", "window.scrollTo(0, (document.documentElement.scrollHeight-innerHeight)/2)")
        wait_for("window.scrollY > 100 && window.scrollY < document.documentElement.scrollHeight-innerHeight-100")

        def modal_wheel(delta, inside=False):
            point = json.loads(browser("eval", "(()=>{const r=document.querySelector('dialog .modal-body').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()")) if inside else {"x":8,"y":300}
            before = json.loads(browser("eval", "window.scrollY"))
            browser("mouse", "move", str(round(point["x"])), str(round(point["y"])))
            browser("mouse", "wheel", str(delta))
            browser("eval", "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))")
            after = json.loads(browser("eval", "window.scrollY"))
            result.setdefault("scroll_observations", []).append({"inside":inside,"delta":delta,"before":before,"after":after})
            assert abs(after-before) <= 1, ("modal scrolled the background", before, after)

        modal_wheel(300)
        modal_wheel(200, inside=True)
        wait_for("document.querySelector('dialog .modal-body').scrollTop > 0")
        browser("eval", "document.querySelector('dialog .modal-body').scrollTop=1e6")
        modal_wheel(600, inside=True)
        browser("eval", "document.querySelector('dialog .modal-body').scrollTop=0")
        modal_wheel(-600, inside=True)
        browser("screenshot", str(record/"modal-scroll-contained.png"))
        button("关闭")
        wait_for("!document.querySelector('dialog[open]')")
        browser("eval", "window.scrollTo(0, (document.documentElement.scrollHeight-innerHeight)/2)")
        unlocked = json.loads(browser("eval", "window.scrollY"))
        browser("mouse", "move", "900", "300")
        browser("mouse", "wheel", "200")
        wait_for("window.scrollY > " + str(unlocked+50))
        checks.append("modal blocks backdrop and edge scroll chaining while its body scrolls, and closing unlocks the page")

        # A retained binding is not proof that its native configuration still exists.
        status_id = harness.rpc("connections.save", {"name":"binding_state","title":"接入状态验收",
            "definition":{"url":harness.url}})["id"]
        harness.changes(status_id, ["hermes:user"])
        hermes_path = harness.home / ".hermes/config.yaml"
        configured = hermes_path.read_text()
        assert "  binding_state:" in configured

        def service_state(expression):
            browser("click", "nav button:first-child")
            button("刷新本机配置")
            wait_for("[...document.querySelectorAll('.service-card')].some(card=>card.querySelector('.service-name strong')?.textContent==='接入状态验收' && (" + expression + "))")

        def account_state(expression):
            button("账号")
            wait_for("[...document.querySelectorAll('.account-card')].some(card=>card.querySelector('.account-heading h2')?.textContent==='接入状态验收' && (" + expression + "))")

        linked_account = "[...card.querySelectorAll('dl > div')].find(row=>row.querySelector('dt')?.textContent==='关联 Agent')?.querySelector('dd')?.textContent"
        service_state("card.querySelector('.agent-tag')?.textContent.includes('Hermes')")
        account_state(linked_account + " === 'Hermes'")
        hermes_path.write_text(configured.replace("  binding_state:", "  moved_binding_state:", 1))
        assert any(b.get("connectionId")==status_id and b.get("missing") for b in harness.rpc("snapshot")["bindings"])
        service_state("card.innerText.includes('尚未接入 Agent')")
        account_state(linked_account + " === '尚未接入'")
        wait_for("[...document.querySelectorAll('.account-card')].some(card=>card.querySelector('.account-heading h2')?.textContent==='接入状态验收' && card.querySelector('.binding-warning')?.innerText.includes('Hermes · 配置缺失'))")
        audit("binding-missing")
        browser("scrollintoview", 'button[aria-label="接入状态验收：Hermes · 配置缺失"]')
        browser("screenshot", str(record/"account-binding-missing.png"))
        service_state("card.querySelector('.binding-warning')?.innerText.includes('Hermes · 配置缺失') && !card.querySelector('.agent-tag')")
        browser("scrollintoview", 'button[aria-label="接入状态验收：Hermes · 配置缺失"]')
        browser("screenshot", str(record/"binding-missing.png"))
        browser("find", "role", "button", "click", "--name", "接入状态验收：Hermes · 配置缺失")
        wait_for("document.querySelector('dialog')?.innerText.includes('配置条目缺失，可清理绑定')")
        browser("press", "Escape")
        checks.append("a removed native entry is excluded from both service and account summaries, with a repair link")

        browser("find", "role", "button", "click", "--name", "接入状态验收：Hermes · 配置缺失")
        button("管理")
        wait_for("document.querySelector('dialog select')?.value==='upsert' && document.querySelector('dialog .target')?.innerText.includes('配置缺失') && !document.querySelector('dialog .target')?.innerText.includes('已有接入')")
        button("预览变更")
        browser("wait", "--text", "核对配置变更")
        button("应用配置")
        browser("wait", "--text", "接入结果")
        button("完成")
        service_state("!card.querySelector('.binding-warning') && card.querySelector('.agent-tag')?.innerText.includes('Hermes')")
        account_state(linked_account + " === 'Hermes' && !card.querySelector('.binding-warning')")
        checks.append("managing a missing binding defaults to reattachment and preview/apply repairs both summaries")
        hermes_path.write_text(configured.replace("  binding_state:", "  moved_binding_state:", 1))

        harness.changes(status_id, ["codex:user"])
        service_state("card.querySelector('.agent-tag')?.innerText.includes('Codex') && !card.querySelector('.agent-tag')?.innerText.includes('Hermes') && card.querySelector('.binding-warning')?.innerText.includes('Hermes · 配置缺失')")
        account_state(linked_account + " === 'Codex'")
        hermes_path.write_text("mcp_servers: [broken\n")
        service_state("card.querySelector('.binding-warning')?.innerText.includes('Hermes · 来源无法读取') && card.querySelector('.agent-tag')?.innerText.includes('Codex')")
        account_state(linked_account + " === 'Codex' && card.querySelector('.binding-warning')?.innerText.includes('Hermes · 来源无法读取')")
        checks.append("remaining native bindings stay visible while missing and unreadable sources are reported separately")

        hermes_path.write_text(configured)
        harness.changes(status_id, ["hermes:user"], action="disable")
        service_state("!card.querySelector('.binding-warning') && [...card.querySelectorAll('.agent-tag')].some(tag=>tag.innerText.includes('Hermes') && tag.innerText.includes('已停用'))")
        account_state(linked_account + "?.includes('Hermes') && !card.querySelector('.binding-warning')")
        audit("binding-status")
        browser("screenshot", str(record/"binding-restored.png"))
        harness.changes(status_id, ["hermes:user", "codex:user"], action="remove")
        harness.rpc("connections.delete", {"id":status_id})
        checks.append("restored disabled configurations remain associated without a stale missing warning")
        for name, title in [("gateway_alpha", "聚合服务甲"), ("gateway_beta", "聚合服务乙")]:
            harness.rpc("connections.save", {"name":name,"title":title,"accountLabel":"隔离账号甲" if name == "gateway_alpha" else "隔离账号乙","definition":{"url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"}}})
        button("刷新本机配置")
        button("统一入口")
        browser("click", ".header-actions button.primary")
        browser("find", "label", "入口名称", "fill", "桌面聚合入口")
        browser("find", "label", "聚合服务甲", "click")
        browser("find", "label", "聚合服务乙", "click")
        browser("find", "first", ".tool-range > button", "click")
        wait_for("document.querySelector('.tool-range-body')?.innerText.includes('沿用服务范围')")
        browser("find", "label", "仅开放所选工具", "click")
        wait_for("!![...document.querySelectorAll('.tool-range-list label')].find(l=>l.innerText==='read_document')")
        browser("find", "label", "read_document", "click")
        audit("gateway-selection")
        checks.append("entry editor selects a per-service allowlist without nesting interactive controls")
        button("保存入口")
        wait_for("document.querySelector('dialog')?.innerText.includes('入口中的服务') && document.querySelectorAll('dialog .binding-detail').length === 2")
        browser("press", "Escape")
        wait_for("document.querySelector('article h2')?.textContent === '桌面聚合入口'")
        button("接给 Agent")
        wait_for("document.querySelectorAll('dialog .mode-choices input').length === 1 && document.querySelector('dialog .mode-choices')?.innerText.includes('统一入口')")
        browser("find", "text", "Hermes · 用户配置", "click")
        button("预览变更")
        browser("wait", "--text", "核对配置变更")
        button("应用配置")
        browser("wait", "--text", "接入结果")
        button("完成")
        native_before = hermes_path.read_bytes()
        button("服务与工具范围")
        browser("find", "label", "聚合服务乙", "click")
        button("保存入口")
        wait_for("document.querySelector('dialog')?.innerText.includes('入口中的服务') && document.querySelectorAll('dialog .binding-detail').length === 2")
        browser("press", "Escape")
        wait_for("!document.querySelector('dialog[open]')")
        assert hermes_path.read_bytes() == native_before
        wait_for("document.querySelector('article')?.innerText.includes('1 项托管服务')")
        browser("screenshot", str(record/"gateway-entry.png"))
        audit("gateways")
        entry = harness.rpc("gateway.list")[0]
        assert entry["members"][0]["includeTools"] == ["read_document"]
        alias = entry["members"][0]["namespace"] + "__read_document"
        import urllib.request
        def stats():
            with urllib.request.urlopen(f"http://127.0.0.1:{harness.port}/stats",timeout=5) as response:
                return json.load(response)
        calls_before = stats()["calls"]
        button("工具 CLI")
        wait_for("!!document.querySelector('.cli-tools button')")
        assert stats()["calls"] == calls_before
        browser("find", "label", "导出 " + alias, "click")
        button("查看 " + alias + " 参数")
        wait_for("!!document.querySelector('.cli-tool-help textarea')")
        wait_for("document.querySelector('.cli-tool-name small')?.innerText.includes('隔离账号甲') && document.querySelector('.cli-source select option:checked')?.innerText.includes('统一入口')")
        wait_for("document.querySelector('.cli-tool-help').getBoundingClientRect().bottom <= document.querySelector('.cli-generation').getBoundingClientRect().top")
        for number in ["9007199254740993", "1e400"]:
            browser("find", "label", "调用参数（JSON 对象）", "fill", '{"id":"weekly-fixture","delay_ms":' + number + '}')
            button("实际调用一次")
            wait_for("document.querySelector('.cli-workflow [role=alert]')?.innerText.includes('精确')")
            assert stats()["calls"] == calls_before
        checks.append("unsafe JSON integers and non-finite numbers are refused in the desktop before any upstream dispatch")
        browser("find", "label", "调用参数（JSON 对象）", "fill", '{"id":"weekly-fixture"}')
        audit("tool-cli")
        browser("screenshot", str(record/"tool-cli.png"))
        browser("eval", "(()=>{const b=[...document.querySelectorAll('dialog button')].find(b=>b.innerText==='实际调用一次');b.click();b.click()})()")
        wait_for("!!document.querySelector('.cli-result') && !document.querySelector('dialog footer .primary').textContent.includes('调用工具')")
        assert stats()["calls"] == calls_before + 1
        browser("find", "label", "命令名称", "fill", "desktop-tools")
        button("选择目录")
        button("生成 CLI")
        wait_for("!!document.querySelector('.cli-generated')")
        manifest = record / "desktop-tools/desktop-tools.mcplink.json"
        assert manifest.is_file() and "synthetic-fixture-authorization" not in manifest.read_text()
        button("复制清单位置")
        wait_for("window.__fixtureClipboard === " + json.dumps(str(manifest)))
        button("检查已有 CLI")
        wait_for("document.querySelector('.cli-contract')?.innerText.includes('工具契约一致')")
        browser("screenshot", str(record/"tool-cli-generated.png"))
        button("关闭")
        checks.append("desktop shows service/account ownership, explains a tool, guards double submission, generates and checks a credential-free CLI with explicit directory selection")
        button("通知 Agent 刷新")
        checks.append("entry exposes an explicit list-refresh action")
        member_id = entry["members"][0]["connectionId"]
        harness.rpc("credentials.update", {"id": member_id, "value": "synthetic-invalid-authorization"})
        button("服务与工具范围")
        browser("find", "first", ".tool-range > button", "click")
        wait_for("!!document.querySelector('.tool-range-body [role=alert]')")
        browser("find", "label", "仅开放所选工具", "click")
        wait_for("document.querySelector('.tool-range-list input')?.checked === true")
        button("保存入口")
        wait_for("document.querySelector('dialog')?.innerText.includes('入口中的服务')")
        browser("press", "Escape")
        assert harness.rpc("gateway.list")[0]["members"][0]["includeTools"] == ["read_document"]
        harness.rpc("credentials.update", {"id": member_id, "value": "synthetic-fixture-authorization"})
        assert harness.rpc("connections.probe", {"id": member_id})["connection"]["status"] == "ready"
        checks.append("a failed tool discovery preserves the entry's saved allowlist through edit and save")
        button("账号")
        wait_for("![...document.querySelectorAll('article h2')].some(h=>h.textContent === '桌面聚合入口') && [...document.querySelectorAll('article')].some(a=>a.querySelector('h2')?.textContent === '聚合服务甲' && a.innerText.includes('Hermes'))")
        browser("click", "nav button:first-child")
        wait_for("![...document.querySelectorAll('.service-name')].some(b=>b.innerText.includes('桌面聚合入口'))")
        checks.append("desktop creates an aggregate entry, configures Hermes once, updates its service view without rewriting native config and attributes shared accounts correctly")
        button("统一入口")
        browser("click", ".header-actions button.primary")
        browser("find", "label", "入口名称", "fill", "异步关闭入口")
        button("保存入口")
        browser("press", "Escape")
        wait_for("document.querySelectorAll('.account-card').length === 2 && !document.querySelector('dialog[open]')")
        checks.append("closing an aggregate editor during an in-flight save refreshes the entry without reopening a dialog")
        result["conclusion"]="PASS"
    finally:
        try:
            if result["conclusion"] != "PASS":
                try:
                    (record/"failure-snapshot.txt").write_text(browser("snapshot"))
                    browser("screenshot", str(record/"failure.png"))
                except subprocess.SubprocessError:
                    pass
            browser("close")
        finally:
            server.shutdown()
            server.server_close()
            harness.close(False)
            shim.unlink(missing_ok=True)
            (record/"report.json").write_text(json.dumps({**result,"checks":checks},indent=2)+"\n")
    print("PASS", record)


if __name__ == "__main__":
    main()
