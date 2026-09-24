/**
 * Crew identity and naming display (ui-redesign-spec, "Identity and naming
 * display rules"). Every Crew surface formats a person through `PersonName` or
 * `personLabel`, and an object through `objectNames`; nothing here ever renders
 * or returns a machine ID.
 */
export { PersonName, type PersonNameProps } from './PersonName';
export {
  agentLabel,
  joinerPerson,
  personFromProjection,
  personLabel,
  personLayout,
  resolvePerson,
  type HandlePlacement,
  type PersonLabelOptions,
  type PersonLayout,
} from './personLabel';
export {
  buildPeopleDirectory,
  usePeopleDirectory,
  type PeopleDirectory,
} from './usePeopleDirectory';
export { cleanName, displayNameKey, isMachineIdShaped, nameKey, stripIgnorable } from './nameKey';
export {
  DISPLAY_NAME_MAX_CHARS,
  displayNameIsUsername,
  isolate,
  personDisplayName,
  sanitizeDisplayText,
  sanitizeUsername,
  usableName,
} from './displayText';
export {
  INSTITUTION_ID_PATTERN,
  institutionId,
  institutionLabel,
  isInstitutionId,
  type KnownInstitution,
} from './institution';
export { InstitutionName, type InstitutionNameProps } from './InstitutionName';
export {
  channelName,
  channelNamesAcrossTeams,
  channelSlug,
  connectionNames,
  connectionServer,
  teamName,
  workspaceName,
  type ChannelNameInput,
  type ConnectionNameInput,
  type TeamNameInput,
} from './objectNames';
export { identityCopy } from './copy';
export {
  PERSON_CONTEXTS,
  type CrewPeopleMap,
  type CrewPeopleMapEntry,
  type CrewPerson,
  type CrewPrincipalInput,
  type DaemonPersonLabel,
  type DaemonPersonLabels,
  type PeopleSnapshotInput,
  type PersonContext,
  type PersonRef,
} from './types';
