// Picking what to hand to an agent: photos, a new photo, files. Images are
// resized (long edge 2560) and re-encoded as JPEG when large or HEIC, so the
// agent on the Mac can read them; other files go as they are (8 MB at most).
import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { camera, cameraPicker } from '@kit.CameraKit';
import { fileIo as fs, picker } from '@kit.CoreFileKit';
import { image } from '@kit.ImageKit';
import { common } from '@kit.AbilityKit';

export const MAX_BYTES = 8 * 1024 * 1024;
const LONG_EDGE = 2560;
const SHRINK_ABOVE = 1536 * 1024;
const IMAGE_EXT = ['jpg', 'jpeg', 'png', 'heic', 'heif', 'webp', 'bmp'];

export class Picked {
  name: string = '';
  bytes: Uint8Array = new Uint8Array(0);
}

export async function pickPhotos(): Promise<string[]> {
  const opts = new photoAccessHelper.PhotoSelectOptions();
  opts.MIMEType = photoAccessHelper.PhotoViewMIMETypes.IMAGE_TYPE;
  opts.maxSelectNumber = 9;
  const r = await new photoAccessHelper.PhotoViewPicker().select(opts);
  return r.photoUris;
}

export async function takePhoto(ctx: common.Context): Promise<string[]> {
  const profile: cameraPicker.PickerProfile = { cameraPosition: camera.CameraPosition.CAMERA_POSITION_BACK };
  const r = await cameraPicker.pick(ctx, [cameraPicker.PickerMediaType.PHOTO], profile);
  return r.resultCode === 0 && r.resultUri ? [r.resultUri] : [];
}

export async function pickFiles(ctx: common.Context): Promise<string[]> {
  const opts = new picker.DocumentSelectOptions();
  opts.maxSelectNumber = 5;
  return await new picker.DocumentViewPicker(ctx).select(opts);
}

function nameOf(uri: string): string {
  const last = uri.split('/').pop() ?? 'file';
  try {
    return decodeURIComponent(last.split('?')[0]);
  } catch (e) {
    return last;
  }
}

function extOf(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot < 0 ? '' : name.substring(dot + 1).toLowerCase();
}

async function shrink(buf: ArrayBuffer): Promise<ArrayBuffer> {
  const src = image.createImageSource(buf);
  try {
    const info = await src.getImageInfo();
    const w = info.size.width;
    const h = info.size.height;
    const k = Math.min(1, LONG_EDGE / Math.max(w, h));
    const opts: image.DecodingOptions = { desiredSize: { width: Math.round(w * k), height: Math.round(h * k) } };
    const pm = await src.createPixelMap(opts);
    const packer = image.createImagePacker();
    const pack: image.PackingOption = { format: 'image/jpeg', quality: 90 };
    const out = await packer.packing(pm, pack);
    pm.release();
    packer.release();
    return out;
  } finally {
    src.release();
  }
}

export async function load(uri: string): Promise<Picked> {
  const f = fs.openSync(uri, fs.OpenMode.READ_ONLY);
  let buf: ArrayBuffer;
  try {
    const size = fs.statSync(f.fd).size;
    buf = new ArrayBuffer(size);
    fs.readSync(f.fd, buf);
  } finally {
    fs.closeSync(f);
  }
  const p = new Picked();
  p.name = nameOf(uri);
  const ext = extOf(p.name);
  if (IMAGE_EXT.includes(ext) && (buf.byteLength > SHRINK_ABOVE || ext === 'heic' || ext === 'heif')) {
    buf = await shrink(buf);
    p.name = p.name.replace(/\.[^.]+$/, '') + '.jpg';
  }
  if (buf.byteLength > MAX_BYTES) {
    throw new Error(`${p.name} 超过 8 MB`);
  }
  p.bytes = new Uint8Array(buf);
  return p;
}
