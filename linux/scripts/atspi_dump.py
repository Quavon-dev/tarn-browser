import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

def walk(node, depth):
    if depth > 40:
        return
    try:
        role = node.get_role_name()
        name = node.get_name() or ""
        text = ""
        try:
            iface = node.get_text_iface()
            if iface:
                text = Atspi.Text.get_text(iface, 0, 60)
        except Exception:
            pass
        print("  " * depth + f"[{role}] {name!r}" + (f" text={text!r}" if text else ""))
        for i in range(node.get_child_count()):
            walk(node.get_child_at_index(i), depth + 1)
    except Exception as err:
        print("  " * depth + f"<error {err}>")

desktop = Atspi.get_desktop(0)
for i in range(desktop.get_child_count()):
    walk(desktop.get_child_at_index(i), 0)
