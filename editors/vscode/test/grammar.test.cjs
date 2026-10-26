const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { test, before } = require('node:test');
const { Registry, parseRawGrammar, INITIAL } = require('vscode-textmate');
const { loadWASM, OnigScanner, OnigString } = require('vscode-oniguruma');

let grammar;
const host = /^host: (.+)$/m.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))[1];
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--offline', '--locked', '--filter-platform', host, '--format-version', '1'], {
    cwd: path.resolve(__dirname, '../../..'), maxBuffer: 20 * 1024 * 1024,
}));
const compilerSyntax = metadata.packages.find(pkg => pkg.name === 'etas_syntax');
const syntaxSource = path.join(path.dirname(compilerSyntax.manifest_path), 'src/token.rs');
const keywords = [...fs.readFileSync(syntaxSource, 'utf8').matchAll(/"(\w+)" => Self::/g)].map(match => match[1]);

before(async () => {
    await loadWASM(fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm')));
    const file = path.resolve(__dirname, '../syntaxes/etas.tmLanguage.json');
    const registry = new Registry({
        onigLib: Promise.resolve({
            createOnigScanner: patterns => new OnigScanner(patterns),
            createOnigString: text => new OnigString(text),
        }),
        loadGrammar: async () => parseRawGrammar(fs.readFileSync(file, 'utf8'), file),
    });
    grammar = await registry.loadGrammar('source.etas');
});

function tokenize(text) {
    let stack = INITIAL;
    return text.split(/\r?\n/).flatMap((line, lineNumber) => {
        const result = grammar.tokenizeLine(line, stack);
        stack = result.ruleStack;
        return result.tokens.map(token => ({
            text: line.slice(token.startIndex, token.endIndex),
            scopes: token.scopes,
            line: lineNumber,
        }));
    });
}

function scope(text, needle) {
    const token = tokenize(text).find(token => token.text === needle);
    assert.ok(token, 'missing token ' + needle + ' in ' + text);
    return token.scopes.join(' ');
}

test('every compiler keyword is highlighted, including standalone/incomplete declarations', () => {
    assert.ok(keywords.length >= 46);
    for (const keyword of keywords) {
        assert.match(scope(keyword, keyword), /keyword\.|storage\.|constant\.language/, keyword);
    }
});

test('declaration names, aliases, annotations, primitives, and spec kinds have scopes', () => {
    assert.match(scope('public flow build() -> string', 'build'), /entity.name.function/);
    assert.match(scope('alias Host = string;', 'Host'), /entity.name.type/);
    assert.match(scope('spec Safe: trace = +Read;', 'trace'), /spec-kind/);
    assert.match(scope('@model("test")', '@model'), /decorator/);
    assert.match(scope('i32 usize unit never', 'usize'), /storage.type.primitive/);
    assert.match(scope('import std.text.join as combine;', 'as'), /keyword/);
});

test('numeric, character, Unicode, string, and comment boundaries', () => {
    for (const value of ['0', '42', '1_024', '0xff', '0o17', '0b101', '3.14', '1e-3']) {
        assert.match(scope(value, value), /constant.numeric/);
    }
    assert.match(scope("'x'", 'x'), /string.quoted.single/);
    assert.match(scope('flow 计算() {}', '计算'), /entity.name.function/);
    const tokens = tokenize('"flow \\"return\\"" // public\n/* type\nalias */\npublicize');
    for (const token of tokens.filter(token => /flow|return|public$|type|alias/.test(token.text))) {
        assert.match(token.scopes.join(' '), /string|comment/);
    }
    assert.doesNotMatch(scope('publicize', 'publicize'), /keyword/);
    assert.match(scope('==', '=='), /keyword.operator/);
    assert.match(scope('->', '->'), /keyword.operator/);
});

const nomo = process.env.ETAS_E2E_PROJECT;
test('all keyword occurrences in the real project receive scopes outside strings/comments', { skip: !nomo }, () => {
    let files = 0;
    let checked = 0;
    for (const file of fs.readdirSync(path.join(nomo, 'src'), { recursive: true })) {
        if (!file.endsWith('.es')) continue;
        files++;
        for (const token of tokenize(fs.readFileSync(path.join(nomo, 'src', file), 'utf8'))) {
            if (token.scopes.some(scope => /^(string|comment)/.test(scope))) continue;
            for (const word of token.text.match(/\b[a-z_]+\b/g) || []) {
                if (!keywords.includes(word)) continue;
                checked++;
                assert.match(token.scopes.join(' '), /keyword\.|storage\.|constant\.language/, file + ':' + (token.line + 1) + ' ' + word);
            }
        }
    }
    assert.ok(files > 0 && checked > 0);
    console.log('TextMate: ' + files + ' project files, ' + checked + ' keyword occurrences checked');
});
