export function matchesToolVersion(output, tool, version) {
  const expected = `${tool} ${version}`;
  return output === expected || output.startsWith(`${expected} `);
}
