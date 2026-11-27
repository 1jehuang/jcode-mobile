// Photo attachments for the composer. UIKit owns the camera and the photo
// library picker, so Swift presents them and hands encoded JPEGs to Rust.
//
// The library uses PHPickerViewController, which runs out of process and needs
// no photo-library permission. The camera needs NSCameraUsageDescription
// (set in scripts/ios-bundle.sh).
import Foundation
#if canImport(UIKit)
import UIKit
import PhotosUI
import AVFoundation
import ImageIO
import UniformTypeIdentifiers

/// status 0: one image (bytes, len, media type). Called once per image.
/// status 1: finished, every image was delivered.
/// status 2: the user cancelled.
/// status 3: unavailable (no camera, permission denied, nothing readable).
public typealias MediaCallback = @convention(c) (Int32, UnsafePointer<UInt8>?, Int, UnsafePointer<CChar>?) -> Void

/// Longest side sent to the model. Larger photos are scaled down first: a
/// 48 MP capture would otherwise be tens of megabytes of base64. 1440px keeps
/// text/screens crisp while fitting the 240 KB budget in 1-2 JPEG passes.
private let maxSide: CGFloat = 1440
private let jpegQuality: CGFloat = 0.78
// Four photos plus base64 and command metadata fit in the sync store's row.
private let maxPhotoBytes = 240 * 1024

private func topController() -> UIViewController? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let window = scenes.flatMap { $0.windows }.first { $0.isKeyWindow } ?? scenes.flatMap { $0.windows }.first
    var top = window?.rootViewController
    while let next = top?.presentedViewController { top = next }
    return top
}

/// Downscales and encodes a `UIImage` (camera capture or fallback) as JPEG
/// within `maxPhotoBytes`. Reuses the downscaled image on each pass instead of
/// re-rasterizing the full-resolution capture.
private func encode(_ image: UIImage) -> Data? {
    let size = image.size
    guard size.width > 0, size.height > 0 else { return nil }
    let format = UIGraphicsImageRendererFormat.default()
    format.scale = 1
    format.opaque = true
    var current = image
    var side = min(maxSide, max(size.width, size.height))
    for _ in 0..<5 {
        let curSize = current.size
        let scale = min(1, side / max(curSize.width, curSize.height))
        let target = CGSize(width: max(1, (curSize.width * scale).rounded()), height: max(1, (curSize.height * scale).rounded()))
        let drawn = UIGraphicsImageRenderer(size: target, format: format).image { ctx in
            UIColor.white.setFill()
            ctx.fill(CGRect(origin: .zero, size: target))
            current.draw(in: CGRect(origin: .zero, size: target))
        }
        current = drawn
        var lastBytes = 0
        for quality in [jpegQuality, 0.58] {
            if let data = drawn.jpegData(compressionQuality: quality) {
                lastBytes = data.count
                if data.count <= maxPhotoBytes { return data }
            }
        }
        let ratio = lastBytes > 0 ? sqrt(Double(maxPhotoBytes) / Double(lastBytes)) : 0.72
        side = max(320, (side * CGFloat(min(0.82, max(0.55, ratio)))).rounded())
    }
    return current.jpegData(compressionQuality: 0.45)
}

/// Hardware-subsampled decode from a photo-library file URL via ImageIO.
/// Avoids decoding a full 12-48 MP HEIC/JPEG bitmap into RAM.
private func encode(fileURL: URL) -> Data? {
    guard let source = CGImageSourceCreateWithURL(fileURL as CFURL, nil) else { return nil }
    let opts: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: Int(maxSide),
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, opts as CFDictionary) else {
        return nil
    }
    return encode(UIImage(cgImage: cgImage, scale: 1, orientation: .up))
}

private final class MediaSession: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate,
    PHPickerViewControllerDelegate, UIAdaptivePresentationControllerDelegate
{
    static var current: MediaSession?
    private let callback: MediaCallback
    private var finished = false

    init(_ callback: @escaping MediaCallback) { self.callback = callback }

    private func deliver(_ data: Data) {
        data.withUnsafeBytes { raw in
            "image/jpeg".withCString { type in
                callback(0, raw.bindMemory(to: UInt8.self).baseAddress, data.count, type)
            }
        }
    }

    /// Exactly one terminal status per session.
    func finish(_ status: Int32) {
        guard !finished else { return }
        finished = true
        callback(status, nil, 0, nil)
        if MediaSession.current === self { MediaSession.current = nil }
    }

    // Camera.
    func imagePickerController(_ picker: UIImagePickerController,
                               didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
        let image = info[.originalImage] as? UIImage
        picker.dismiss(animated: true)
        DispatchQueue.global(qos: .userInitiated).async {
            let data = image.flatMap(encode)
            DispatchQueue.main.async {
                if let data { self.deliver(data); self.finish(1) } else { self.finish(3) }
            }
        }
    }

    func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
        picker.dismiss(animated: true)
        finish(2)
    }

    // Photo library.
    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard !results.isEmpty else { finish(2); return }
        var encoded = [Data?](repeating: nil, count: results.count)
        let lock = NSLock()
        let group = DispatchGroup()
        for (i, result) in results.enumerated() {
            let provider = result.itemProvider
            if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                group.enter()
                provider.loadFileRepresentation(forTypeIdentifier: UTType.image.identifier) { url, _ in
                    let data = url.flatMap(encode(fileURL:))
                    lock.lock(); encoded[i] = data; lock.unlock()
                    group.leave()
                }
            } else if provider.canLoadObject(ofClass: UIImage.self) {
                group.enter()
                provider.loadObject(ofClass: UIImage.self) { object, _ in
                    let data = (object as? UIImage).flatMap(encode)
                    lock.lock(); encoded[i] = data; lock.unlock()
                    group.leave()
                }
            }
        }
        group.notify(queue: .main) {
            // Keep the order the user selected them in.
            let ready = encoded.compactMap { $0 }
            for data in ready { self.deliver(data) }
            self.finish(ready.isEmpty ? 3 : 1)
        }
    }

    // Swipe-to-dismiss on either sheet.
    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        finish(2)
    }
}

private func present(_ source: Int32, _ limit: Int, _ session: MediaSession) {
    guard let top = topController() else { session.finish(3); return }
    if source == 0 {
        guard UIImagePickerController.isSourceTypeAvailable(.camera) else { session.finish(3); return }
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.mediaTypes = ["public.image"]
        picker.delegate = session
        top.present(picker, animated: true)
        picker.presentationController?.delegate = session
    } else {
        var config = PHPickerConfiguration()
        config.filter = .images
        config.selectionLimit = max(1, limit)
        let picker = PHPickerViewController(configuration: config)
        picker.delegate = session
        top.present(picker, animated: true)
        picker.presentationController?.delegate = session
    }
}
#endif

/// 1 when this device has a usable camera (0 on the simulator).
@_cdecl("jcode_media_camera_available")
public func jcode_media_camera_available() -> Int32 {
    #if canImport(UIKit)
    return UIImagePickerController.isSourceTypeAvailable(.camera) ? 1 : 0
    #else
    return 0
    #endif
}

#if canImport(UIKit)
/// Presents the camera (`source` 0) or the photo library (`source` 1, up to
/// `limit` photos). Returns 0 when nothing could be presented, in which case
/// the callback is never called. Call on the main thread.
@_cdecl("jcode_media_pick")
public func jcode_media_pick(_ source: Int32, _ limit: Int32, _ callback: MediaCallback?) -> Int32 {
    guard let callback, MediaSession.current == nil else { return 0 }
    let session = MediaSession(callback)
    MediaSession.current = session
    if source == 0 {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .denied, .restricted:
            session.finish(3)
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { granted in
                DispatchQueue.main.async {
                    if granted { present(source, 1, session) } else { session.finish(3) }
                }
            }
        default:
            present(source, 1, session)
        }
    } else {
        present(source, Int(limit), session)
    }
    return 1
}
#endif
