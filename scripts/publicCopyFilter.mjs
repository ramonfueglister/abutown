const retiredPublicEntries = new Set([`open${'gfx2'}-classic`, 'simutrans-assets']);

export function shouldCopyPublicEntry(entryName) {
  return !isMacOsMetadata(entryName) && !retiredPublicEntries.has(entryName);
}

function isMacOsMetadata(entryName) {
  return entryName === '.DS_Store' || entryName.startsWith('._');
}
