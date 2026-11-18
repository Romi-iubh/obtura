#!/usr/bin/python3
# A stand-in for the top bar's status-icon host (Ubuntu's AppIndicator extension), for the
# smoke tests: it owns org.kde.StatusNotifierWatcher and writes each registration to the log
# file given as argument. The test then reads the item and clicks its menu with gdbus.
import sys

from gi.repository import Gio, GLib

XML = """<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
</interface></node>"""

items = []
log = open(sys.argv[1], "a")


def call(conn, sender, path, iface, method, params, invocation):
    if method == "RegisterStatusNotifierItem":
        items.append(params.unpack()[0])
        log.write("registered %s %s\n" % (sender, params.unpack()[0]))
        log.flush()
    invocation.return_value(None)


def prop(conn, sender, path, iface, name):
    return {
        "RegisteredStatusNotifierItems": GLib.Variant("as", items),
        "IsStatusNotifierHostRegistered": GLib.Variant("b", True),
        "ProtocolVersion": GLib.Variant("i", 0),
    }[name]


def bus_acquired(conn, name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object("/StatusNotifierWatcher", node.interfaces[0], call, prop, None)


Gio.bus_own_name(Gio.BusType.SESSION, "org.kde.StatusNotifierWatcher", Gio.BusNameOwnerFlags.NONE, bus_acquired, None, None)
GLib.MainLoop().run()
