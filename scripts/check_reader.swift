// Native WebKit verification for a generated synthetic reader page. Run with:
// swift scripts/check_reader.swift INPUT.html OUTPUT.png
import AppKit
import WebKit

final class ReaderCheck: NSObject, WKNavigationDelegate {
    let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 900, height: 460))
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 460), styleMask: [.borderless], backing: .buffered, defer: false)
    let input: URL
    let output: URL

    init(input: URL, output: URL) {
        self.input = input
        self.output = output
        super.init()
        window.contentView = view
        window.setFrameOrigin(NSPoint(x: -10000, y: -10000))
        window.orderFront(nil)
        view.navigationDelegate = self
        view.loadFileURL(input, allowingReadAccessTo: input.deletingLastPathComponent())
    }

    func fail(_ error: String) {
        fputs("Reader verification failed: \(error)\n", stderr)
        exit(1)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        fail(error.localizedDescription)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        fail(error.localizedDescription)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        webView.evaluateJavaScript("JSON.stringify({images:Array.from(document.images).map(i=>({src:i.getAttribute('src'),width:i.naturalWidth,height:i.naturalHeight})),links:document.querySelectorAll('a[href]').length,active:document.querySelectorAll('script,iframe,object,embed').length})") { value, error in
            guard error == nil, let text = value as? String,
                  let data = text.data(using: .utf8),
                  let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let images = result["images"] as? [[String: Any]],
                  images.contains(where: { ($0["width"] as? Int) == 160 && ($0["height"] as? Int) == 80 }),
                  images.allSatisfy({ ($0["src"] is NSNull) || ($0["src"] as? String)?.hasPrefix("data:image/png;base64,") == true }),
                  result["links"] as? Int == 0, result["active"] as? Int == 0 else {
                self.fail("Unexpected rendered resources: \(String(describing: value)), \(String(describing: error))")
                return
            }
            webView.takeSnapshot(with: nil) { image, error in
                guard error == nil, let tiff = image?.tiffRepresentation,
                      let bitmap = NSBitmapImageRep(data: tiff),
                      let png = bitmap.representation(using: .png, properties: [:]) else {
                    self.fail("WebKit snapshot failed")
                    return
                }
                do { try png.write(to: self.output) }
                catch { self.fail(error.localizedDescription) }
                print("Verified native WebKit: 160×80 local PNG, no active elements or navigation; \(self.output.path)")
                exit(0)
            }
        }
    }
}

guard CommandLine.arguments.count == 3 else { fatalError("Usage: check_reader.swift INPUT.html OUTPUT.png") }
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let check = ReaderCheck(input: URL(fileURLWithPath: CommandLine.arguments[1]), output: URL(fileURLWithPath: CommandLine.arguments[2]))
DispatchQueue.main.asyncAfter(deadline: .now() + 30) { check.fail("Timed out") }
app.run()
