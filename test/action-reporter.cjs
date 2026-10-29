module.exports = class ActionReporter {
  onBegin(_config, suite) { this.suite = suite; }
  onEnd() {
    const records = this.suite.allTests().map(test => ({
      id: `browser-${test.title.match(/^S\d+[a-z]?/)?.[0] ?? test.id}`,
      action: test.title,
      status: test.results.at(-1)?.status === 'passed' ? 'PASS'
        : test.results.at(-1)?.status === 'skipped' || !test.results.length ? 'NOT RUN' : 'FAIL'
    }));
    console.log(`BROWSER_RESULTS ${JSON.stringify(records)}`);
  }
};
