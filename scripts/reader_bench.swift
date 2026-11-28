// Persistent offscreen WebKit peer for the macOS reader benchmark.
// Input lines contain only URLs produced by the isolated benchmark archive.
import AppKit
import WebKit

final class Bench: NSObject, WKNavigationDelegate {
    let window: NSWindow
    let view: WKWebView
    var output: URL?
    var start: UInt64 = 0
    override init() {
        let bounds = NSRect(x: 0, y: 0, width: 720, height: 480)
        view = WKWebView(frame: bounds)
        window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        super.init()
        window.appearance = NSAppearance(named: .aqua)
        window.contentView = view
        window.setFrameOrigin(NSPoint(x: -10000, y: -10000))
        window.orderFront(nil)
        view.navigationDelegate = self
    }
    func next() {
        guard let line = readLine() else { exit(0) }
        guard let input = try? JSONDecoder().decode([String].self, from: Data(line.utf8)),
              input.count == 2, let file = URL(string: input[0]), file.isFileURL else { exit(1) }
        output = input[1].isEmpty ? nil : URL(fileURLWithPath: input[1])
        start = DispatchTime.now().uptimeNanoseconds
        let current = start
        DispatchQueue.main.asyncAfter(deadline: .now() + 10) {
            if self.start == current { exit(7) }
        }
        view.loadFileURL(file, allowingReadAccessTo: file.deletingLastPathComponent())
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        view.takeSnapshot(with: nil) { image, _ in
            let us = (DispatchTime.now().uptimeNanoseconds - self.start) / 1000
            guard let image else { exit(2) }
            if let output = self.output {
                guard let data = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: data),
                      let png = bitmap.representation(using: .png, properties: [:]) else { exit(3) }
                do { try png.write(to: output) } catch { exit(4) }
            }
            FileHandle.standardOutput.write(Data("\(us)\n".utf8))
            DispatchQueue.main.async { self.next() }
        }
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { exit(5) }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { exit(6) }
}
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let bench = Bench()
DispatchQueue.main.async { bench.next() }
app.run()
