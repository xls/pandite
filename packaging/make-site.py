"""Build the privacy and support pages of the Pandite website from PRIVACY.md and SUPPORT.md.
Usage: python packaging/make-site.py . www
The folder www/ is the website. Publish: copy its content to the root of the
gh-pages branch and push it."""
import html
import re
import shutil
import sys
from pathlib import Path

repo, out = Path(sys.argv[1]), Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)

CSS = """
:root { color-scheme: dark; --bg: #0b1020; --panel: #121a30; --text: #e9ecf4; --dim: #a7b0c6; --accent: #4f9cff; --line: #243052; }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text); font: 17px/1.6 -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; }
main { max-width: 760px; margin: 0 auto; padding: 104px 20px 72px; }
.bar { position: fixed; top: 0; left: 0; right: 0; z-index: 5; display: flex; align-items: center; gap: 12px; padding: 12px clamp(16px, 4vw, 40px); backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px); background: rgba(7, 11, 24, 0.55); border-bottom: 1px solid var(--line); }
.bar a.home { display: flex; align-items: center; gap: 12px; color: var(--text); text-decoration: none; }
.bar img { width: 30px; height: 30px; }
.bar b { font-size: 18px; letter-spacing: 0.2px; }
.bar nav { margin-left: auto; display: flex; gap: clamp(12px, 2.4vw, 26px); font-size: 15px; }
.bar nav a { color: var(--dim); text-decoration: none; transition: color 0.2s; }
.bar nav a:hover, .bar nav a.here { color: var(--text); }
@media (max-width: 860px) { .bar nav a.hide { display: none; } }
h1 { font-size: 34px; line-height: 1.2; margin: 0 0 12px; }
h2 { font-size: 21px; margin: 36px 0 8px; }
p, li { color: var(--dim); }
strong { color: var(--text); }
a { color: var(--accent); }
code { background: var(--panel); border: 1px solid var(--line); border-radius: 5px; padding: 1px 6px; font: 14px/1.4 ui-monospace, Consolas, monospace; color: var(--text); }
pre { background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 14px 16px; overflow-x: auto; }
pre code { border: 0; padding: 0; background: none; }
.hero { text-align: center; padding: 16px 0 8px; }
.hero img { width: 148px; height: 148px; }
.hero p { font-size: 20px; max-width: 560px; margin: 8px auto 26px; }
.button { display: inline-block; background: var(--accent); color: #06122b; text-decoration: none; font-weight: 600; padding: 12px 22px; border-radius: 10px; margin: 4px 6px; }
.button.quiet { background: none; color: var(--text); border: 1px solid var(--line); }
.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 14px; margin-top: 34px; }
.card { background: var(--panel); border: 1px solid var(--line); border-radius: 12px; padding: 16px 18px; }
.card h3 { margin: 0 0 6px; font-size: 17px; color: var(--text); }
.card p { margin: 0; font-size: 15px; }
footer { margin-top: 56px; padding-top: 18px; border-top: 1px solid var(--line); font-size: 14px; color: var(--dim); }
footer a { color: var(--dim); }
"""


def inline(text):
    text = html.escape(text, quote=False)
    text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", lambda m: f'<a href="{link(m.group(2))}">{m.group(1)}</a>', text)
    text = re.sub(r"&lt;(https?://[^&]+)&gt;", r'<a href="\1">\1</a>', text)
    return text


def link(target):
    if target == "PRIVACY.md":
        return "privacy.html"
    if target == "SUPPORT.md":
        return "support.html"
    if target.startswith("docs/"):
        return "https://github.com/xls/pandite/blob/main/" + target
    return target


def markdown(text):
    lines = text.replace("\r\n", "\n").split("\n")
    parts, para, items, kind, code = [], [], [], None, None

    def flush():
        nonlocal para, items, kind
        if para:
            parts.append("<p>" + inline(" ".join(para)) + "</p>")
            para = []
        if items:
            parts.append(f"<{kind}>" + "".join(f"<li>{inline(i)}</li>" for i in items) + f"</{kind}>")
            items, kind = [], None

    for line in lines:
        if code is not None:
            if line.strip().startswith("```"):
                parts.append("<pre><code>" + html.escape("\n".join(code)) + "</code></pre>")
                code = None
            else:
                code.append(line)
            continue
        if line.strip().startswith("```"):
            flush()
            code = []
            continue
        m = re.match(r"^(#{1,3}) (.*)$", line)
        if m:
            flush()
            n = len(m.group(1))
            parts.append(f"<h{n}>{inline(m.group(2))}</h{n}>")
            continue
        m = re.match(r"^(\d+\.|-) (.*)$", line)
        if m:
            if para:
                flush()
            want = "ol" if m.group(1) != "-" else "ul"
            if kind and kind != want:
                flush()
            kind = want
            items.append(m.group(2))
            continue
        if line.startswith("  ") and items and line.strip():
            items[-1] += " " + line.strip()
            continue
        if not line.strip():
            flush()
            continue
        para.append(line.strip())
    flush()
    return "\n".join(parts)


def page(title, body, description, name):
    here_support = ' class="here"' if name == "support.html" else ""
    here_privacy = ' class="here"' if name == "privacy.html" else ""
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="{description}">
<link rel="icon" href="icon.png">
<style>{CSS}</style>
</head>
<body>
<header class="bar">
<a class="home" href="./"><img src="icon.png" alt=""><b>Pandite</b></a>
<nav><a class="hide" href="./#features">Features</a><a class="hide" href="./#install">Install</a><a href="support.html"{here_support}>Support</a><a href="privacy.html"{here_privacy}>Privacy</a><a href="https://github.com/xls/pandite">GitHub</a></nav>
</header>
<main>
{body}
<footer>Pandite is free and open source under the MIT license. <a href="privacy.html">Privacy policy</a> - <a href="support.html">Support</a> - <a href="https://github.com/xls/pandite">Source code</a></footer>
</main>
</body>
</html>
"""


# The welcome page, index.html, is written by hand and lives in www/.
for name, src, title, desc in [
    ("privacy.html", "PRIVACY.md", "Privacy policy - Pandite", "Pandite collects no data and makes no network connection."),
    ("support.html", "SUPPORT.md", "Support - Pandite", "How to get help with Pandite."),
]:
    body = markdown((repo / src).read_text(encoding="utf8"))
    (out / name).write_text(page(title, body, desc, name), encoding="ascii", newline="\n")
shutil.copyfile(repo / "packaging/linux/icons/hicolor/256x256/apps/pandite.png", out / "icon.png")
(out / ".nojekyll").write_text("", encoding="ascii")
print("wrote", sorted(p.name for p in out.iterdir()))
