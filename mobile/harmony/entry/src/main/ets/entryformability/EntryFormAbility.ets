// The home-screen card's provider (docs/harmony/DESIGN.md §6b). Runs in its own
// process: it only records which cards exist and what they show first; the app
// (while running) and the bridge (through Push Kit) fill them afterwards.
import { formBindingData, FormExtensionAbility, formInfo } from '@kit.FormKit';
import { Want } from '@kit.AbilityKit';
import { CARD_KEYS, Cards } from '../core/Cards';

export default class EntryFormAbility extends FormExtensionAbility {
  onAddForm(want: Want): formBindingData.FormBindingData {
    const id = (want.parameters?.[formInfo.FormParam.IDENTITY_KEY] as string) ?? '';
    const values = Cards.added(this.context, id);
    values['formId'] = id;
    const binding = formBindingData.createFormBindingData(values);
    // Push Kit card messages may refresh these keys (bridge/src/card.rs).
    binding.proxies = CARD_KEYS.map((k: string): formBindingData.ProxyData => ({ key: k, subscriberId: id }));
    return binding;
  }

  onRemoveForm(formId: string): void {
    Cards.removed(this.context, formId);
  }

  onUpdateForm(formId: string): void {
  }

  onFormEvent(formId: string, message: string): void {
  }

  onAcquireFormState(want: Want): formInfo.FormState {
    return formInfo.FormState.READY;
  }
}
