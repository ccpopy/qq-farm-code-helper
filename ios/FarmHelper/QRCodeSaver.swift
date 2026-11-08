import Foundation
import ImageIO
import Photos
import UIKit

enum QRCodeError: LocalizedError {
    case invalidImage
    case permission
    case saveFailed

    var errorDescription: String? {
        switch self {
        case .invalidImage: return "服务器未返回有效的二维码图片"
        case .permission: return "未允许添加照片，无法保存二维码"
        case .saveFailed: return "二维码保存失败"
        }
    }
}

enum QRCodeSaver {
    static func decode(_ data: Data) throws -> UIImage {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any],
              let width = properties[kCGImagePropertyPixelWidth as String] as? Int,
              let height = properties[kCGImagePropertyPixelHeight as String] as? Int,
              width >= 1, height >= 1, width <= 4096, height <= 4096,
              let image = UIImage(data: data) else {
            throw QRCodeError.invalidImage
        }
        return image
    }

    static func save(_ image: UIImage) async throws {
        let status: PHAuthorizationStatus = await withCheckedContinuation { continuation in
            PHPhotoLibrary.requestAuthorization(for: .addOnly) { value in
                continuation.resume(returning: value)
            }
        }
        guard status == .authorized || status == .limited else {
            throw QRCodeError.permission
        }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            PHPhotoLibrary.shared().performChanges {
                PHAssetChangeRequest.creationRequestForAsset(from: image)
            } completionHandler: { success, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if success {
                    continuation.resume(returning: ())
                } else {
                    continuation.resume(throwing: QRCodeError.saveFailed)
                }
            }
        }
    }
}
