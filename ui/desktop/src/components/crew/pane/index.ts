/**
 * The details pane (ui-redesign-spec, "The details pane" and "Ask my agent"): the one non-modal
 * `<aside>` beside the conversation, its About and Members tabs, Ask my agent and its model
 * picker. Files, Access and Chat access arrive through `DetailsPane`'s slots from their own areas.
 * Every component reads the controller through `useCrew()`; nothing here imports another area.
 */
export { DetailsPane, type DetailsPaneProps } from './DetailsPane';
export { AboutTab, type AboutTabProps } from './AboutTab';
export { MembersTab, type MembersTabProps } from './MembersTab';
export { AgentTaskPane, type AgentTaskPaneProps } from './AgentTaskPane';
export { CrewModelPicker, ModelTierMarks, type CrewModelPickerProps } from './CrewModelPicker';
export {
  providerLabel,
  useConfiguredModels,
  type ConfiguredModels,
  type ModelChoice,
} from './useConfiguredModels';
export { aboutCopy, agentCopy, membersCopy, paneCopy, unknownOutcomeCopy } from './copy';
