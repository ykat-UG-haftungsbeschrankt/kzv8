function hotAdd(value) {
  return value + 1;
}

%PrepareFunctionForOptimization(hotAdd);
for (let index = 0; index < 10000; index++) {
  hotAdd(index);
}

%OptimizeFunctionOnNextCall(hotAdd);
const result = hotAdd(41);
const status = %GetOptimizationStatus(hotAdd);
const optimizedStatusBit = 1 << 3;

if ((status & optimizedStatusBit) === 0) {
  throw new Error(`Function did not optimize; status=${status}`);
}
if (result !== 42) {
  throw new Error(`Unexpected result: ${result}`);
}

print(`JIT optimized; result=${result}, status=${status}`);