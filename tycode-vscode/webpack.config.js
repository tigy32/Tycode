const path = require('path');
const CopyPlugin = require('copy-webpack-plugin');

const extensionConfig = {
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
            loader: 'ts-loader',
            options: {
              configFile: 'tsconfig.json'
            }
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
        { from: 'src/webview/*.css', to: 'webview/[name][ext]' }
      ]
    })
  ],
  devtool: 'nosources-source-map',
  infrastructureLogging: {
    level: 'log'
  }
};

const webviewConfig = {
  target: 'web',
  mode: 'none',
  entry: './src/webview/main.ts',
  output: {
    path: path.resolve(__dirname, 'out/webview'),
    filename: 'main.js'
  },
  resolve: {
    extensions: ['.ts', '.js'],
    extensionAlias: {
      '.js': ['.ts', '.js']
    }
  },
  module: {
    rules: [
      {
        test: /\.ts$/,
        exclude: /node_modules/,
        use: [
          {
            loader: 'ts-loader',
            options: {
              configFile: 'tsconfig.webview.json'
            }
          }
        ]
      }
    ]
  },
  devtool: 'nosources-source-map'
};

const settingsConfig = {
  target: 'web',
  mode: 'none',
  entry: './src/webview/settings.js',
  output: {
    path: path.resolve(__dirname, 'out/webview'),
    filename: 'settings.js'
  },
  resolve: {
    extensions: ['.ts', '.js'],
    extensionAlias: {
      '.js': ['.ts', '.js']
    }
  },
  module: {
    rules: [
      {
        test: /\.ts$/,
        exclude: /node_modules/,
        use: [
          {
            loader: 'ts-loader',
            options: {
              configFile: 'tsconfig.webview.json'
            }
          }
        ]
      }
    ]
  },
  devtool: 'nosources-source-map'
};

module.exports = [extensionConfig, webviewConfig, settingsConfig];