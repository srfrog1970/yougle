const path = require('path');
const { getDefaultConfig, mergeConfig } = require('@react-native/metro-config');

/**
 * Metro configuration
 * https://reactnative.dev/docs/metro
 *
 * The example app consumes `yougle-native` (the library in `..`) as a
 * local, unpublished package — it's not in node_modules, so Metro's
 * default resolver can't find it on its own. `watchFolders` lets Metro
 * see source changes there; `extraNodeModules` is what actually makes
 * `import ... from 'yougle-native'` resolve.
 *
 * @type {import('@react-native/metro-config').MetroConfig}
 */
const root = path.resolve(__dirname, '..');

const config = {
  watchFolders: [root],
  resolver: {
    extraNodeModules: {
      'yougle-native': root,
    },
  },
};

module.exports = mergeConfig(getDefaultConfig(__dirname), config);
