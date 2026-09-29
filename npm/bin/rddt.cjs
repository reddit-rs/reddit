#!/usr/bin/env node
"use strict";

const { execute } = require("../lib/cli.cjs");
const result = execute(process.argv.slice(2));
if (result.error) {
  console.error(`rddt: ${result.error.message}`);
  process.exitCode = 1;
} else if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exitCode = result.status ?? 1;
}
