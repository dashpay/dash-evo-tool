"""Export only passive gallery markup; no active content or external URLs."""

import html
import re
from html.parser import HTMLParser

from logic import CAPTURE_NAME


class PassiveHTML(HTMLParser):
    tags = set("h1 h2 h3 p pre code table tr th td ul li b strong em img a br".split())

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.output = []
        self.hidden = 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style", "iframe", "object", "svg", "math", "template"):
            self.hidden += 1
        if self.hidden or tag not in self.tags:
            return
        safe = []
        for key, value in attrs:
            if value is None:
                continue
            if (key == "id" and re.fullmatch(r"[a-zA-Z0-9_-]+", value)) or (
                key == "alt" and tag == "img"
            ):
                safe.append((key, value))
            elif (tag, key) in (("img", "src"), ("a", "href")) and re.fullmatch(
                rf"annotated/(?:clean/)?{CAPTURE_NAME}\.png|#[a-zA-Z0-9_-]+|data:image/png;base64,[A-Za-z0-9+/=]+",
                value,
            ):
                safe.append((key, value))
        self.output.append(
            "<"
            + tag
            + "".join(
                f' {key}="{html.escape(value, quote=True)}"' for key, value in safe
            )
            + ">"
        )

    def handle_endtag(self, tag):
        if tag in ("script", "style", "iframe", "object", "svg", "math", "template"):
            self.hidden = max(0, self.hidden - 1)
        elif not self.hidden and tag in self.tags and tag not in ("img", "br"):
            self.output.append(f"</{tag}>")

    def handle_data(self, data):
        if not self.hidden:
            self.output.append(html.escape(data))


def sanitize_html(text, *, trusted_css=""):
    parser = PassiveHTML()
    parser.feed(text)
    parser.close()
    return (
        '<!doctype html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width,initial-scale=1">'
        + (f"<style>{trusted_css}</style>" if trusted_css else "")
        + "</head><body>"
        + "".join(parser.output)
        + "</body></html>"
    )
