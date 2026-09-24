/**
 * Crew's dialogs and confirmations (ui-redesign-spec, "Dialog inventory", "Progressive
 * disclosure", "Privacy and institution"). A layout mounts `CrewDialogs` once and opens any of
 * these through the controller's `openDialog(intent)`; each dialog is also exported for a surface
 * that owns its own open state.
 */
export { CrewDialogs, dialogKey, HOSTED_DIALOG_KINDS, type CrewDialogsProps } from './CrewDialogs';
export { AddPeopleDialog, type AddPeopleDialogProps } from './AddPeopleDialog';
export {
  CrewConfirmation,
  MakeConnectionPublicDialog,
  type CrewConfirmationProps,
  type MakeConnectionPublicDialogProps,
} from './confirmations';
export {
  advancedInUse,
  ConnectionSettingsDialog,
  type ConnectionSettingsDialogProps,
} from './ConnectionSettingsDialog';
export { CreateChannelDialog, type CreateChannelDialogProps } from './CreateChannelDialog';
export { CreateTeamDialog, type CreateTeamDialogProps } from './CreateTeamDialog';
export { DeviceCodeInput, type DeviceCodeInputProps } from './DeviceCodeInput';
export { EditProfileDialog, type EditProfileDialogProps } from './EditProfileDialog';
export { InvitePeopleDialog, type InvitePeopleDialogProps } from './InvitePeopleDialog';
export { KeysDialog, type KeysDialogProps } from './KeysDialog';
export { LetInDialog, type LetInDialogProps } from './LetInDialog';
export { MakePrivateDialog, type MakePrivateDialogProps } from './MakePrivateDialog';
export { PersonPicker, type PersonPickerProps } from './PersonPicker';
export { RenameDialog, type RenameDialogProps } from './RenameDialog';
export { SharePathDialog, type SharePathDialogProps } from './SharePathDialog';
export {
  TransferOwnershipDialog,
  type TransferOwnershipDialogProps,
} from './TransferOwnershipDialog';
export {
  WorkspaceSettingsDialog,
  type WorkspaceSettingsDialogProps,
} from './WorkspaceSettingsDialog';
export {
  DEVICE_CODE_LENGTH,
  deviceCodeProblem,
  groupDeviceCodeInput,
  normalizeDeviceCodeInput,
} from './deviceCode';
export { channelSlugPreview, channelSlugProblem, INSTITUTION_FIELD_PATTERN } from './nameRules';
export { uniqueNamesSupported, workspaceLabelFor, workspacePhraseFor } from './workspace';
export * from './copy';
