# クイックスタート

10 分以内に最初のレポートまで進みます。リポジトリにある合成の例を使います。サポートの問い合わせ 300 件を、プロンプトの 2 つの版（`triage-v1`、`triage-v2`）で振り分けた記録で、参照ラベルは 2 人の評価者が付けています。英語版は [quickstart.md](quickstart.md) で、どちらも同じ手順を説明しています。

## 1. インストール

Rust 1.90 以降で次を実行します。

```sh
cargo install --locked evalsig-cli
```

ビルド済みのバイナリ、Python の wheel、Homebrew の formula は [README](../README.ja.md#インストール) にあります。

## 2. 例を取得する

```sh
git clone https://github.com/kechol/evalsig
mkdir triage
cd triage
cp ../evalsig/examples/synthetic/records.jsonl .
```

`records.jsonl` は evalsig が読む記録（`records/1`）です。2 つの run、item、参照ラベル、run ごと item ごとに 1 つの判定が入っています。

## 3. プロジェクトを作って取り込む

```sh
evalsig init
evalsig import records.jsonl
```

`init` は既定の `config.toml` を入れた `.evalsig/` を作ります。`import` は記録をスナップショットとして保存し（名前は中身のハッシュ）、台帳に 1 行書きます。この後の結果はすべて、どのスナップショットから計算したかを名指しします。

## 4. 最初のレポートを読む

```sh
evalsig report triage-v1
```

最初に読むところは次のとおりです。

- `metrics`：正解率、ヒット率、誤警報率、適合率です。それぞれに値（`value`）、95% の区間（`ci`）、区間の方法があります。item はスレッド（`cluster`）に属するので、区間はスレッド単位で再標本化します（`bootstrap`）。
- `n_missing` と `missing_bounds`：6 件の判定が時間切れで欠けています。この範囲は、欠けた判定を最悪の値と最良の値で数えたときの指標です。
- `reference`：一部の item には 2 人がラベルを付けているので、参照は `soft` です（item ごとに、陽性と言った評価者の割合で数えます）。`rater_spread` は、評価者 1 人ずつを参照にしたときの指標の範囲です。
- `sdt`：弁別力 d′ と判定基準 c です。「見分ける力」と「どこで陽性と言い始めるか」を分けて示します。

## 5. 2 つの版を比べる

```sh
evalsig compare triage-v1 triage-v2
```

最初の行が verdict です。例は `improved: accuracy: difference +0.0533 (better is positive; 95% interval +0.0133 to +0.0933)` です。`improved` になるのは、対応のある差の区間が 0 を含まず、差が `min_effect` 以上のときだけです。そうでなければ `negligible`、`indistinguishable`、`worsened`、`insufficient_data` のどれかになります。`transition` は正解と不正解の間で動いた item を数えます。終了コードは verdict に従います（[CLI の参照](cli.md#exit-codes)）。

## 6. ブラウザーで見る

```sh
evalsig dashboard
```

ダッシュボードは 127.0.0.1 だけで待ち受け、起動ごとに作るトークンつきの URL を表示します。プロジェクトを読むだけで、書き込みはしません。Ctrl-C で止めます。`evalsig report triage-v1 --format html > report.html` は、同じレポートを外部を読み込まない 1 枚のページとして書き出します。

## 次に進む

- 自分の評価を取り込む：Inspect のログは `evalsig import --adapter inspect <log.eval>`、promptfoo の出力やローカルのデータベースは `--adapter promptfoo` で取り込みます。それ以外は `evalsig map probe`、`map draft`、`map test` で `mapping.toml` を書きます。取り込みの経路の一覧は `evalsig recipes acquire` で見られます。
- 必要な item の数を見積もる：`evalsig power` と [sample-size.md](sample-size.md)（英語）。
- 封印した保留テストでリリースを判定する：[referee.md](referee.md)（英語）。
- コーディングエージェントに evalsig を使わせる：`evalsig skill install` と [agent-interface.md](agent-interface.md)（英語）。
- すべての指標と区間の定義は [metrics.md](metrics.md)、仕様の全体は [spec.md](spec.md) にあります（英語）。
