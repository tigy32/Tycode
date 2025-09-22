const path = require('path');
const CopyPlugin = require('copy-webpack-plugin');

const config = {
  target: 'node',
  mode: 'none',
  entry: './src/extension.ts',
  output: {
    path: path.resolve(__dirname, 'out'),
    filename: 'extension.js',
    libraryTarget: 'commonjs2'
  },
  externals: {
    vscode: 'commonjs vscode'
  },
  resolve: {
    extensions: ['.ts', '.js', '.wasm']
  },
  module: {
    rules: [
      {
        test: /\.ts$/,
        exclude: /node_modules/,
        use: [
          {
            loader: 'ts-loader'
          }
        ]
      }
    ]
  },
  plugins: [
    new CopyPlugin({
      patterns: [
        { from: 'wasm/*.wasm', to: '[name][ext]' },
        { from: 'wasm/*.js', to: '[name][ext]' },
        { from: 'src/webview/*.html', to: 'webview/[name][ext]' },
        { from: 'src/webview/*.css', to: 'webview/[name][ext]' },
        { from: 'src/webview/*.js', to: 'webview/[name][ext]' }
      ]
    })
  ],
  devtool: 'nosources-source-map',
  infrastructureLogging: {
    level: 'log'
  }
};

module.exports = config;