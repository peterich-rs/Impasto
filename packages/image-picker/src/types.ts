export type MediaType = "image" | "video" | "any";
export type ImageOutputFormat = "original" | "jpeg" | "png" | "webp";
export type PreferredCamera = "back" | "front";
export type VideoQuality = "low" | "medium" | "high";

export type ImagePickerErrorCode =
  | "permission-denied"
  | "camera-unavailable"
  | "unsupported-media"
  | "not-supported"
  | "cancelled"
  | "busy"
  | "outside-scope"
  | "host-unavailable"
  | "io";

export interface ImageProcessOptions {
  /** 最大宽高约束（等比缩小，不放大原图） */
  readonly maxWidth?: number;
  readonly maxHeight?: number;
  /** 1..=100，仅对有损导出（jpeg / webp）生效，默认 85 */
  readonly quality?: number;
  readonly outputFormat?: ImageOutputFormat; // 默认 "original"（若需缩放但未改格式，HEIC/BMP 等会安全转为 jpeg）
  /** 自动读取 EXIF Orientation 并旋转像素归正，默认 true */
  readonly fixOrientation?: boolean;
  /** 默认 true：在重新编码输出时剔除 EXIF GPS 位置与拍摄设备隐私元数据 */
  readonly stripMetadata?: boolean;
}

export interface PickMediaOptions {
  readonly mediaType?: MediaType; // 默认 "image"
  readonly multiple?: boolean; // 默认 false
  /** multiple == true 时允许选择的最大数量，默认无上限/系统限制 */
  readonly maxSelection?: number;
  readonly process?: ImageProcessOptions;
  readonly outputScope?: "cache" | "temp"; // 默认 "cache"
}

export interface CapturePhotoOptions {
  readonly preferredCamera?: PreferredCamera; // 默认 "back"
  readonly process?: ImageProcessOptions;
  readonly outputScope?: "cache" | "temp";
}

export interface CaptureVideoOptions {
  readonly preferredCamera?: PreferredCamera;
  readonly maxDurationSec?: number;
  readonly quality?: VideoQuality; // 默认 "medium"
  readonly outputScope?: "cache" | "temp";
}

export interface PickedMedia {
  readonly kind: "image" | "video";
  /** 相对于 `outputScope` 的安全路径，如 "image-picker/2b9f...jpg"，可直接传给 @impasto/file-system 或 @impasto/http */
  readonly relativePath: string;
  readonly scope: "cache" | "temp";
  readonly fileName: string;
  readonly mimeType: string;
  readonly size: number;
  readonly width: number | null;
  readonly height: number | null;
  readonly durationMs: number | null;
}
