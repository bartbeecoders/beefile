import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Bar button for the BeeFile window. The file manager stays a real window;
// this only focuses it or starts it.
BarWidget {
  id: root
  moduleName: "bart.beefile"

  function pluginFile(name) {
    var url = Qt.resolvedUrl(name).toString()
    if (url.indexOf("file://") === 0)
      url = url.slice(7)
    return decodeURIComponent(url)
  }

  function binaryPath() {
    var custom = String(setting("binary", "") || "")
    if (custom !== "")
      return custom
    var home = String(Quickshell.env("HOME") || "")
    return home + "/.local/bin/beefile"
  }

  function openApp() {
    var argv = [pluginFile("launch.sh"), binaryPath()]
    var start = String(setting("startPath", "") || "")
    if (start !== "")
      argv.push(start)
    Util.execArgv(argv)
  }

  IpcHandler {
    target: "bart.beefile"

    function app(): string {
      root.openApp()
      return "ok"
    }

    function open(): string {
      root.openApp()
      return "ok"
    }
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: ""
    tooltipText: "BeeFile"
    onPressed: function(mouseButton) {
      if (mouseButton === Qt.LeftButton)
        root.openApp()
    }
  }
}
