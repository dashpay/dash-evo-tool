"""Publish complete controller state to readers with an atomic rename."""

import os
from tempfile import NamedTemporaryFile


def atomic_write(path, text):
    with NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as output:
        temporary = output.name
        try:
            output.write(text)
            output.close()
            os.replace(temporary, path)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)
