/**
 * The channel area (ui-redesign-spec, "The channel header and channel menu" and "Where errors
 * render: exactly once"): the 44px channel band with its menu, and the connection bar under it.
 * Every component reads the controller through `useCrew()`; nothing here imports another area.
 */
export { ChannelHeader, type ChannelHeaderProps } from './ChannelHeader';
export { ChannelMenu, type ChannelMenuProps } from './ChannelMenu';
export { AgentAccessChip, type AgentAccessChipProps } from './AgentAccessChip';
export { MemberStack, type MemberStackProps } from './MemberStack';
export { ConnectionBar, type ConnectionBarProps } from './ConnectionBar';
export { channelCopy, connectionBarCopy } from './copy';
export { activeTasksIn } from './presentation';
export {
  readSeenDevices,
  seenDevicesKey,
  useNewDeviceNotice,
  writeSeenDevices,
  type NewDeviceNotice,
} from './useNewDeviceNotice';
