// The GPUI capture leaves the native WebKit reader blank. This renders a page
// the capture generated, so `just screenshots` can place it in that space.
// swift scripts/reader_shot.swift INPUT.html OUTPUT.png WIDTH HEIGHT
import AppKit
import WebKit

final class ReaderShot: NSObject, WKNavigationDelegate {
    let view: WKWebView
    let window: NSWindow
    let output: URL

    init(input: URL, output: URL, size: NSSize) {
        let frame = NSRect(origin: .zero, size: size)
        view = WKWebView(frame: frame)
        window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
        self.output = output
        super.init()
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = view
        window.setFrameOrigin(NSPoint(x: -10000, y: -10000))
        window.orderFront(nil)
        view.navigationDelegate = self
        view.loadFileURL(input, allowingReadAccessTo: input.deletingLastPathComponent())
    }

    func fail(_ message: String) -> Never {
        fputs("Reader shot failed: \(message)\n", stderr)
        exit(1)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        let configuration = WKSnapshotConfiguration()
        configuration.snapshotWidth = NSNumber(value: Double(view.frame.width))
        view.takeSnapshot(with: configuration) { image, error in
            guard let image, let tiff = image.tiffRepresentation,
                let bitmap = NSBitmapImageRep(data: tiff),
                let png = bitmap.representation(using: .png, properties: [:])
            else { self.fail(error?.localizedDescription ?? "no image") }
            do { try png.write(to: self.output) } catch { self.fail(error.localizedDescription) }
            exit(0)
        }
    }
}

let arguments = CommandLine.arguments
guard arguments.count == 5, let width = Double(arguments[3]), let height = Double(arguments[4]) else {
    fputs("Usage: reader_shot.swift INPUT.html OUTPUT.png WIDTH HEIGHT\n", stderr)
    exit(2)
}
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let shot = ReaderShot(
    input: URL(fileURLWithPath: arguments[1]),
    output: URL(fileURLWithPath: arguments[2]),
    size: NSSize(width: width, height: height))
app.run()
