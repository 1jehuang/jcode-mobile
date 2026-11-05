// Share extension: collects the shared text and URLs and hands them to the
// app as jcode://new?text=<...>, which opens a new session with that prompt.
//
// No app group is used (App Groups cannot be registered through the App Store
// Connect API, see docs/IOS_EXTENSIONS.md), so the URL scheme carries the
// payload. Extensions cannot call UIApplication.open directly, so the request
// walks the responder chain to the host application, the standard approach
// for share extensions that hand off to their containing app.
import UIKit
import UniformTypeIdentifiers

@objc(ShareViewController)
final class ShareViewController: UIViewController {
    private var done = false

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !done else { return }
        done = true
        Task { await forward() }
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .clear
    }

    private func forward() async {
        var parts: [String] = []
        let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
        for item in items {
            if let t = item.attributedContentText?.string, !t.isEmpty { parts.append(t) }
            for provider in item.attachments ?? [] {
                if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
                   let url = try? await provider.loadItem(forTypeIdentifier: UTType.url.identifier) as? URL {
                    parts.append(url.absoluteString)
                } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier),
                          let text = try? await provider.loadItem(forTypeIdentifier: UTType.plainText.identifier) as? String {
                    parts.append(text)
                }
            }
        }
        var seen = Set<String>()
        let text = parts.filter { seen.insert($0).inserted }.joined(separator: "\n\n")
        var comps = URLComponents()
        comps.scheme = "jcode"
        comps.host = "new"
        if !text.isEmpty {
            // Keep the URL well under iOS limits; the composer shows the rest.
            comps.queryItems = [URLQueryItem(name: "text", value: String(text.prefix(8000)))]
        }
        if let url = comps.url { openHost(url) }
        extensionContext?.completeRequest(returningItems: nil, completionHandler: nil)
    }

    private func openHost(_ url: URL) {
        var responder: UIResponder? = self
        while let r = responder {
            if let app = r as? UIApplication {
                app.open(url, options: [:], completionHandler: nil)
                return
            }
            responder = r.next
        }
        // Older hosts: fall back to the legacy selector.
        let sel = sel_registerName("openURL:")
        responder = self
        while let r = responder {
            if r.responds(to: sel) { _ = r.perform(sel, with: url); return }
            responder = r.next
        }
    }
}
