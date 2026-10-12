import { toastService } from '../../../toasts';
import { agentAddExtension, ExtensionConfig, agentRemoveExtension } from '../../../api';
import { errorMessage } from '../../../utils/conversionUtils';
import { userActionHeaders } from '../../../utils/userAction';
import { showCrossAffiliationNotice } from '../../../utils/crossAffiliationNotice';
import { notifySessionToolsChanged } from '../../../utils/sessionToolEvents';
import {
  createExtensionRecoverHints,
  formatExtensionErrorMessage,
} from '../../../utils/extensionErrorUtils';

export async function addToAgent(
  extensionConfig: ExtensionConfig,
  sessionId: string,
  showToast: boolean
) {
  const extensionName = extensionConfig.name;
  let toastId = showToast
    ? toastService.loading({
        title: extensionName,
        msg: 'Adding extension…',
      })
    : 0;

  try {
    const attached = await agentAddExtension({
      // Issue #56 Task 58: attaching tools to a private chat needs the
      // proof-of-user. This does NOT relax the DR-16 refusal beside it — a
      // private extension on a public chat is still refused outright, and no
      // header changes that.
      headers: await userActionHeaders(),
      body: { session_id: sessionId, config: extensionConfig },
      throwOnError: true,
    });
    notifySessionToolsChanged(sessionId);
    if (showToast) {
      toastService.dismiss(toastId);
      toastService.success({
        title: extensionName,
        msg: 'Extension added',
      });
    }
    // Issue #56 DR-26 — the user's enable path, and the surface where the ruling
    // was unimplemented rather than merely quiet. The daemon detected a
    // cross-institutional mismatch here from the beginning and wrote it only to
    // `tracing::warn!`; the person attaching another institution's connector saw
    // the green toast above and nothing else. The body is the daemon's own
    // statement, naming both institutions, and it is empty in the normal case.
    //
    // ⚠ **Shown regardless of `showToast`.** That flag suppresses progress and
    // success chatter for bulk/background attaches; a privacy statement is not
    // chatter, and silencing one because a caller asked for a quiet install is
    // how this surface went dark in the first place. Every caller in the tree
    // passes `true` today, so the choice is currently invisible — which is
    // exactly when it needs writing down.
    //
    // ⚠ **After the success toast, so it is the one left on screen.** It does not
    // auto-close; the success toast does.
    showCrossAffiliationNotice(attached.data);
  } catch (error) {
    if (showToast) {
      toastService.dismiss(toastId);
      const errMsg = errorMessage(error);
      const recoverHints = createExtensionRecoverHints(errMsg);
      const msg = formatExtensionErrorMessage(errMsg, 'Failed to add extension');
      toastService.error({
        title: extensionName,
        msg: msg,
        traceback: errMsg,
        recoverHints,
      });
    }
    throw error;
  }
}

export async function removeFromAgent(
  extensionName: string,
  sessionId: string,
  showToast: boolean
) {
  let toastId = showToast
    ? toastService.loading({
        title: extensionName,
        msg: 'Removing extension…',
      })
    : 0;

  try {
    await agentRemoveExtension({
      headers: await userActionHeaders(),
      body: { session_id: sessionId, name: extensionName },
      throwOnError: true,
    });
    notifySessionToolsChanged(sessionId);
    if (showToast) {
      toastService.dismiss(toastId);
      toastService.success({
        title: extensionName,
        msg: 'Extension removed',
      });
    }
  } catch (error) {
    if (showToast) {
      toastService.dismiss(toastId);
      const errMsg = errorMessage(error);
      const msg = formatExtensionErrorMessage(errMsg, 'Failed to remove extension');
      toastService.error({
        title: extensionName,
        msg: msg,
        traceback: errMsg,
      });
    }
    throw error;
  }
}

export function sanitizeName(name: string) {
  return name.toLowerCase().replace(/-/g, '').replace(/_/g, '').replace(/\s/g, '');
}
