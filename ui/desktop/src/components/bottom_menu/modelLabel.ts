/**
 * The model's name as a person says it (spec 3.7): the release-date stamp a
 * provider appends comes off, so `gpt-5.6-sol-2026-07-09` reads `gpt-5.6-sol`
 * and `claude-3-5-sonnet-20241022` reads `claude-3-5-sonnet`. The full id stays
 * in the chip's tooltip, its accessible name and the menu.
 *
 * Only a trailing date is removed, and only when something is left in front of
 * it: a name that IS a date, or has none, comes back unchanged.
 *
 * A Bedrock id carries more than a date: an inference profile's region
 * (`us.`, `eu.`, `apac.`, `global.`), the vendor (`anthropic.`) and the API
 * version (`-v1:0`, after which a date is no longer trailing). Those come off
 * too, so `us.anthropic.claude-sonnet-4-6` reads `claude-sonnet-4-6` and
 * `anthropic.claude-3-5-sonnet-20240620-v1:0` reads `claude-3-5-sonnet`. A
 * name left as one word keeps its vendor (`deepseek.r1-v1:0` reads
 * `deepseek-r1`, not `r1`).
 *
 * The version rule is Bedrock's only: elsewhere `-v3` is a generation
 * (`deepseek-v3`), and the vendor segment is recognised by name unless a
 * region prefix already says the id is Bedrock's, so `qwen2.5` or
 * `llama3.1:8b` are never read as one.
 */
const TRAILING_DATE = /-(?:\d{4}-\d{2}-\d{2}|\d{8})$/;

const BEDROCK_REGION = '(?:us|us-gov|eu|apac|jp|au|ca|global)';
const BEDROCK_VENDOR =
  '(?:amazon|anthropic|ai21|cohere|deepseek|google|luma|meta|minimax|mistral|moonshot|nvidia|openai|qwen|stability|twelvelabs|writer)';
const BEDROCK_ID = new RegExp(
  `^(?:${BEDROCK_REGION}\\.([a-z][a-z0-9-]*)|(${BEDROCK_VENDOR}))\\.(.+)$`
);
const BEDROCK_VERSION = /-(?:v\d+(?::\d+)?|\d+:\d+)$/;

function withoutDate(name: string): string {
  const stripped = name.replace(TRAILING_DATE, '');
  return stripped.length > 0 ? stripped : name;
}

export function friendlyModelName(id: string): string {
  const trimmed = id.trim();
  const bedrock = BEDROCK_ID.exec(trimmed);
  if (!bedrock) return withoutDate(trimmed);

  const vendor = bedrock[1] ?? bedrock[2];
  const model = bedrock[3];
  const unversioned = model.replace(BEDROCK_VERSION, '');
  const name = withoutDate(unversioned.length > 0 ? unversioned : model);
  return name.includes('-') ? name : `${vendor}-${name}`;
}
