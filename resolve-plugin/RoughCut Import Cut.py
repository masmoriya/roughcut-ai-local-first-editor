# RoughCut Import Cut — DaVinci Resolve script (works in the FREE version).
#
# Upstream MCP bridge. Requires an explicit RoughCut project UUID.
#
# Why this exists: the free Resolve cannot be driven by outside apps
# (external scripting is Studio-only), but scripts launched from Resolve's
# own menu have full API access. Same hand-off, triggered from this side.
#
# The RoughCut app must be running. If it asks to approve the export, click
# Approve in the RoughCut window (external edits always ask).

import json
import os
import stat
import sys
import tempfile
import urllib.request
import urllib.parse


def data_dir():
    return os.path.expanduser("~/Library/Application Support/roughcut")


def discover_endpoint():
    path = os.path.join(data_dir(), "mcp.json")
    if not os.path.exists(path):
        raise RuntimeError(
            "RoughCut does not appear to be running (no endpoint file at %s). "
            "Launch the RoughCut app and try again." % path
        )
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, encoding="utf-8") as discovery:
        metadata = os.fstat(discovery.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o600:
            raise RuntimeError("RoughCut discovery must be a private regular file owned by the current user.")
        content = discovery.read(4097)
        if len(content) > 4096:
            raise RuntimeError("RoughCut discovery file is too large.")
        info = json.loads(content)
    url = info["url"]
    endpoint = urllib.parse.urlsplit(url)
    port = endpoint.port
    if not port or url != "http://127.0.0.1:%d/mcp" % port:
        raise RuntimeError("Invalid RoughCut loopback endpoint.")
    token = info.get("token")
    if not isinstance(token, str) or len(token) != 40 or not token.isascii() or not token.isalnum():
        raise RuntimeError("Invalid RoughCut discovery token.")
    return url, token


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


LOOPBACK_HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())


def call_tool(url, token, name, arguments):
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
    ).encode()
    req = urllib.request.Request(
        url,
        data=body,
        headers={"content-type": "application/json", "authorization": "Bearer " + token},
    )
    with LOOPBACK_HTTP.open(req, timeout=600) as resp:
        out = json.load(resp)
    payload = json.loads(out["result"]["content"][0]["text"])
    if isinstance(payload, dict) and payload.get("error"):
        raise RuntimeError(payload["error"].get("message", "tool error"))
    return payload


def get_resolve():
    r = globals().get("resolve")
    if r:
        return r
    try:
        import DaVinciResolveScript as dvr  # type: ignore

        return dvr.scriptapp("Resolve")
    except Exception:
        return None


def main():
    if len(sys.argv) != 2:
        print("Choose a RoughCut project explicitly with its UUID, or export XML manually in RoughCut.")
        return
    project_id = sys.argv[1]
    url, token = discover_endpoint()
    listed = call_tool(url, token, "list_projects", {})
    projects = listed.get("projects", [])
    if not projects:
        print("RoughCut has no projects — edit something there first.")
        return
    project = next((item for item in projects if item.get("id") == project_id), None)
    if project is None:
        print("No RoughCut project matches the selected UUID.")
        return
    print("Importing the current cut of %r…" % project["name"])
    print("(if RoughCut asks to approve the export, click Approve there)")

    xml_path = os.path.join(tempfile.mkdtemp(prefix="roughcut-import-"), "timeline.xml")
    exported = call_tool(
        url,
        token,
        "export",
        {"project_id": project["id"], "target": "resolve_xml", "out_path": xml_path},
    )
    xml_path = exported.get("path", xml_path)

    r = get_resolve()
    if not r:
        print("Exported XML: %s (run this from Resolve's Scripts menu to auto-import)" % xml_path)
        return
    pm = r.GetProjectManager()
    proj = pm.GetCurrentProject() if pm else None
    if not proj:
        print("Open (or create) a Resolve project first.")
        return
    tl = proj.GetMediaPool().ImportTimelineFromFile(xml_path)
    if tl:
        print("Imported timeline %r — media linked from the original file." % tl.GetName())
    else:
        print("Resolve refused the import; the XML is at %s" % xml_path)


if __name__ == "__main__":
    main()
