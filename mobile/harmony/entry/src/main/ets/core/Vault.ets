// The paired Mac and its device token, kept in Asset Store Kit (encrypted by
// the system), never in plain preferences.
import { asset } from '@kit.AssetStoreKit';
import { fromUtf8, utf8 } from './Bytes';

const ALIAS = 'ddup.machine.v1';

export interface MachineData {
  name: string;
  hosts: string[];
  port: number;
  key: string;
  deviceId: string;
  token: string;
}

function aliasQuery(): asset.AssetMap {
  const q: asset.AssetMap = new Map();
  q.set(asset.Tag.ALIAS, utf8(ALIAS));
  return q;
}

export async function loadMachine(): Promise<MachineData | null> {
  const q = aliasQuery();
  q.set(asset.Tag.RETURN_TYPE, asset.ReturnType.ALL);
  try {
    const found: asset.AssetMap[] = await asset.query(q);
    if (found.length === 0) {
      return null;
    }
    const secret = found[0].get(asset.Tag.SECRET) as Uint8Array;
    return JSON.parse(fromUtf8(secret)) as MachineData;
  } catch (e) {
    // 24000002: not found.
    return null;
  }
}

export async function forgetMachine(): Promise<void> {
  try {
    await asset.remove(aliasQuery());
  } catch (e) {
    // nothing stored
  }
}

export async function saveMachine(m: MachineData): Promise<void> {
  await forgetMachine();
  const attr = aliasQuery();
  attr.set(asset.Tag.SECRET, utf8(JSON.stringify(m)));
  attr.set(asset.Tag.ACCESSIBILITY, asset.Accessibility.DEVICE_FIRST_UNLOCKED);
  await asset.add(attr);
}
