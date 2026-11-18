"""Exercise the real frontend builder, HTTP server, resource graph and live reload."""
import argparse
from datetime import datetime, timezone
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
BUILDER = ROOT / "scripts/frontend.mjs"
# Synthetic loopback requests must not use the host's external proxy settings.
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class Assets(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.urls = []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "script" and "src" in attrs:
            assert attrs.get("type") == "module"
            self.urls.append(attrs["src"])
        if tag == "link" and attrs.get("rel") == "stylesheet":
            self.urls.append(attrs["href"])


def stop(process):
    if process and process.poll() is None:
        if os.name == "nt":
            process.terminate()
        else:
            os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True)
            else:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    fixture = record / "fixture"
    (fixture / "src").mkdir(parents=True)
    (fixture / "public").mkdir()
    (fixture / "index.html").write_text('<html><head><title>fixture</title></head><body><script type="module" src="/src/main.tsx"></script></body></html>')
    (fixture / "src/main.tsx").write_text('import "./style.css"; document.body.dataset.fixture = "before-watch";')
    (fixture / "src/style.css").write_text('@font-face{font-family:fixture;src:url("./font.woff2")}body{color:blue}')
    (fixture / "src/font.woff2").write_bytes(b"synthetic-font-resource")
    (fixture / "public/icon.txt").write_text("public fixture")
    (fixture / "private-marker.txt").write_text("must not be served")
    result = {"conclusion": "FAIL", "checked_at": datetime.now(timezone.utc).isoformat(), "checks": []}
    process = stream = None
    log = (record / "frontend.log").open("w")
    try:
        subprocess.run(["node", str(BUILDER)], cwd=fixture, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=30)
        html = (fixture / "dist/index.html").read_text()
        assets = Assets(html).urls
        assert len(assets) == 2
        for asset in assets:
            file = fixture / "dist" / asset.lstrip("/")
            assert file.is_file(), asset
            if asset.endswith(".js"):
                assert "before-watch" in file.read_text() and "/esbuild" not in file.read_text()
            else:
                urls = re.findall(r"url\(['\"]?([^)'\"]+)", file.read_text())
                assert urls and all((file.parent / url).is_file() for url in urls)
        assert (fixture / "dist/icon.txt").read_text() == "public fixture"
        assert not list((fixture / "dist").rglob("*.map"))
        result["checks"].append("production module, stylesheet, font and public assets resolve; no live reload or production sourcemaps")

        with socket.socket() as available:
            available.bind(("127.0.0.1", 0))
            port = available.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        process = subprocess.Popen(["node", str(BUILDER), "--dev", "--port", str(port)], cwd=fixture,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=os.name != "nt")
        deadline = time.monotonic() + 20
        while True:
            if process.poll() is not None:
                raise AssertionError("Development server exited before readiness")
            try:
                with HTTP.open(origin + "/", timeout=1) as response:
                    dev_html = response.read().decode()
                break
            except (OSError, urllib.error.URLError) as error:
                if time.monotonic() >= deadline:
                    raise AssertionError(f"Development server did not become ready: {error}")
                time.sleep(0.1)
        js_url = next(u for u in Assets(dev_html).urls if u.endswith(".js"))
        with HTTP.open(origin + js_url, timeout=3) as response:
            assert 'EventSource("/esbuild")' in response.read().decode()
        for path in ["/private-marker.txt", "/src/main.tsx"]:
            try:
                HTTP.open(origin + path, timeout=3)
                raise AssertionError("Project source/private file was exposed")
            except urllib.error.HTTPError as error:
                assert error.code == 404
        result["checks"].append("loopback development entry serves generated output without exposing source or private files")

        stream = HTTP.open(urllib.request.Request(origin + "/esbuild", headers={"Accept": "text/event-stream"}), timeout=10)
        assert stream.headers["Content-Type"].startswith("text/event-stream")
        events = queue.Queue()

        def read_events():
            try:
                for line in stream:
                    if line.startswith(b"data:"):
                        events.put(line.decode())
            except (OSError, ValueError):
                pass

        threading.Thread(target=read_events, daemon=True).start()
        (fixture / "src/main.tsx").write_text('import "./style.css"; document.body.dataset.fixture = "after-watch";')
        changed = events.get(timeout=10)
        assert "/assets/index.js" in changed
        with HTTP.open(origin + js_url, timeout=3) as response:
            assert "after-watch" in response.read().decode()
        result["checks"].append("source change rebuilds the served module and emits a live reload event")

        html_file = fixture / "index.html"
        html_file.write_text(html_file.read_text().replace("<title>fixture</title>", "<title>template changed</title>"))
        changed = events.get(timeout=10)
        assert "/assets/index.js" in changed
        with HTTP.open(origin + "/", timeout=3) as response:
            assert "<title>template changed</title>" in response.read().decode()
        result["checks"].append("an HTML-only change also emits reload and updates the served document")

        occupied = record / "occupied-fixture"
        shutil.copytree(fixture / "src", occupied / "src")
        shutil.copy2(fixture / "index.html", occupied / "index.html")
        denied = subprocess.run(["node", str(BUILDER), "--dev", "--port", str(port)], cwd=occupied,
                                capture_output=True, text=True, timeout=15)
        assert denied.returncode != 0 and "mcplink frontend:" not in denied.stdout
        result["checks"].append("an occupied explicit port fails instead of selecting a different port")
        stop(process)
        stream.close()
        stream = None
        deadline = time.monotonic() + 10
        while True:
            with socket.socket() as released:
                if released.connect_ex(("127.0.0.1", port)) != 0:
                    break
            if time.monotonic() >= deadline:
                raise AssertionError("Development server retained its port after exit")
            time.sleep(0.1)
        result["checks"].append("stopping the owned server releases its listener")
        result["conclusion"] = "PASS"
    except Exception as error:
        result["error"] = str(error) or type(error).__name__
    finally:
        if stream:
            stream.close()
        stop(process)
        log.close()
        (record / "report.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    raise SystemExit(0 if result["conclusion"] == "PASS" else 1)


if __name__ == "__main__":
    main()
