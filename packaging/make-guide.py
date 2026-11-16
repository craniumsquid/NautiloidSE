#!/usr/bin/env python3
"""Makes the user guide that ships with the app: docs/HOW-TO-USE.md as one HTML page, with
its pictures and the app's fonts inside it, and from that a PDF.

    python3 packaging/make-guide.py <out folder> [--browser PATH]

Writes "How to use Nautiloid.html" and, with a Chrome or Chromium to print it, "How to use
Nautiloid.pdf". Run from the repository root. Needs Python-Markdown
(pip install markdown==3.7).
"""

import argparse
import base64
import html
import pathlib
import re
import shutil
import subprocess
import sys

import markdown

ROOT = pathlib.Path(__file__).resolve().parent.parent
GUIDE = ROOT / "docs" / "HOW-TO-USE.md"
NAME = "How to use Nautiloid"

FONTS = [
    ("Barlow", 400, "Barlow-Regular.ttf"),
    ("Barlow", 500, "Barlow-Medium.ttf"),
    ("Barlow", 600, "Barlow-SemiBold.ttf"),
    ("Barlow", 700, "Barlow-Bold.ttf"),
    ("Cinzel", 700, "Cinzel-Bold.ttf"),
]

# Light, for reading and for printing; the pictures carry the app's own dark look.
STYLE = """
@page { size: A4; margin: 16mm 15mm; }
:root { --text: #241d14; --muted: #6b5d48; --accent: #8a6420; --line: #ddd2bf;
        --panel: #f6f1e8; }
html { background: #fff; }
body { font-family: Barlow, "Segoe UI", sans-serif; font-size: 11.5pt; line-height: 1.5;
       color: var(--text); max-width: 46em; margin: 2em auto; padding: 0 1.2em; }
h1, h2 { font-family: Cinzel, Georgia, serif; font-weight: 700; color: #1a140c;
         line-height: 1.2; }
h1 { font-size: 26pt; margin: 0 0 0.2em; }
h2 { font-size: 17pt; margin: 1.6em 0 0.5em; padding-bottom: 0.2em;
     border-bottom: 2px solid var(--accent); }
h3 { font-size: 13pt; font-weight: 700; margin: 1.3em 0 0.4em; }
.version { color: var(--muted); margin: 0 0 1.5em; }
a { color: var(--accent); }
strong { font-weight: 700; }
code { font-family: Consolas, "DejaVu Sans Mono", monospace; font-size: 0.88em;
       background: var(--panel); padding: 0.05em 0.15em; border-radius: 3px; }
img { display: block; width: 100%; height: auto; margin: 0.8em 0 1.2em;
      border: 1px solid #000; border-radius: 6px; break-inside: avoid; }
table { border-collapse: collapse; width: 100%; margin: 0.8em 0; font-size: 10pt; }
th, td { border: 1px solid var(--line); padding: 0.35em 0.5em; text-align: left;
         vertical-align: top; }
th { background: var(--panel); }
td code { word-break: break-all; }
li { margin: 0.2em 0; }
p, li { orphans: 3; widows: 3; }
h2, h3 { break-after: avoid; }
@media print {
  body { max-width: none; margin: 0; padding: 0; }
  /* A heading stays with what follows it, and a sentence with the picture it leads to. */
  p:has(+ p > img), ul:has(+ p > img), ol:has(+ p > img) { break-after: avoid; }
}
"""


def data_uri(path: pathlib.Path, kind: str) -> str:
    return f"data:{kind};base64," + base64.b64encode(path.read_bytes()).decode("ascii")


def version() -> str:
    found = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)
    return found.group(1) if found else ""


def page() -> str:
    source = GUIDE.read_text(encoding="utf-8")
    body = markdown.markdown(source, extensions=["tables", "toc", "sane_lists"])

    # The pictures, inside the page.
    def picture(match: re.Match) -> str:
        path = (GUIDE.parent / html.unescape(match.group(1))).resolve()
        return f'src="{data_uri(path, "image/png")}"'

    body = re.sub(r'src="([^"]+\.png)"', picture, body)
    # The version under the title.
    body = body.replace(
        "</h1>", f'</h1>\n<p class="version">Nautiloid Save Editor {version()}</p>', 1
    )
    fonts = "\n".join(
        f'@font-face {{ font-family: {family}; font-weight: {weight}; '
        f'src: url("{data_uri(ROOT / "assets" / "fonts" / file, "font/ttf")}"); }}'
        for family, weight, file in FONTS
    )
    return (
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
        f"<title>{NAME}</title>\n"
        '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
        f"<style>\n{fonts}\n{STYLE}</style>\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    )


def find_browser(given: str | None) -> str | None:
    if given:
        return given
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        found = shutil.which(name)
        if found:
            return found
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", type=pathlib.Path)
    parser.add_argument("--browser", help="Chrome or Chromium, for the PDF")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    page_path = args.out / f"{NAME}.html"
    page_path.write_text(page(), encoding="utf-8")
    print(page_path)

    browser = find_browser(args.browser)
    if not browser:
        print("No Chrome or Chromium found, so no PDF", file=sys.stderr)
        return 1
    pdf_path = args.out / f"{NAME}.pdf"
    subprocess.run(
        [
            browser,
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-pdf-header-footer",
            f"--print-to-pdf={pdf_path.resolve()}",
            page_path.resolve().as_uri(),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if not pdf_path.is_file() or pdf_path.stat().st_size == 0:
        print("The browser wrote no PDF", file=sys.stderr)
        return 1
    print(pdf_path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
