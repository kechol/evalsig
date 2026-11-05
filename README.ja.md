# evalsig

[English](README.md)

evalsig は、毎回同じ答えを返すとは限らない判定器（LLM の評価器、分類器、ルールの集まり）の評価記録を読み、次の問いに区間つきで答えます。

- 人が陽性と呼ぶものと陰性と呼ぶものを、判定器がどれだけ見分けられるか。どこで「陽性」と言い始めるか。
- 同じ入力をもう一度判定させたとき、どれくらい答えが割れるか。
- その変更は本当に効いたのか。判定の線が動いただけか、誤差の範囲か。

分析専用のコマンドラインツールで、ローカルの Web ダッシュボードが付きます。AI のモデルは一切呼びません。人の参照ラベルもぶれるものとして扱います。ぶれの大きさを測り、推定が何を対象にしているかを示し、システムと人を同じ規則で比べます。

AI エージェントがループでシステムを改善するときには、evalsig を審判（referee）として使えます。保留テストのラベルをエージェントから隠して封印し、使うたびに数え、人が前もって承認した条件で合否を決めます。

## 状態

リリース前です。コマンドラインの仕様、記録の形式、出力の封筒は作っている途中で、1.0 までは変わることがあります。

## インストール

ソースから入れるには、Rust 1.90 以降（最低限サポートする Rust の版）で次を実行します。

```sh
cargo install --locked evalsig-cli
```

同じバイナリを、Python の wheel（`pip install evalsig` で `evalsig` が PATH に入ります。`crates/evalsig-cli/pyproject.toml` から maturin でビルドします）と、リリースのソースからビルドする Homebrew の formula（`packaging/homebrew/evalsig.rb`）でも配っています。

macOS と Linux（aarch64 と x86_64）のビルド済みバイナリは、GitHub の各 Release に付きます。アーカイブごとの SHA-256 チェックサムと、シェルのインストーラーも付きます。どのアーカイブにも GitHub の artifact attestation が付いています。審判として信頼する前に、どこから来たものかを確かめてください。

```sh
gh attestation verify evalsig-cli-aarch64-apple-darwin.tar.xz --repo kechol/evalsig
```

## クイックスタート

[docs/quickstart.ja.md](docs/quickstart.ja.md) では、空のディレクトリから 10 分以内にレポートと比較まで進みます。仕様は [docs/spec.md](docs/spec.md)、指標の定義は [docs/metrics.md](docs/metrics.md)、すべてのコマンドと option は [docs/cli.md](docs/cli.md) にあります（いずれも英語）。

## 答える問い

| 問い | コマンド |
|---|---|
| 出してよいか | `gate`（封印した保留テストで、事前登録した条件を判定）、`check`（区間つきの絶対基準） |
| 変更は効いたか | `compare`（対応のある区間、最小の効果、移り変わりの表、対の勝率） |
| 見分ける力が上がったのか、線が動いただけか | `report`（ROC と AUC、d′ と判定基準 c） |
| 人の意見の割れを含めるとどれくらい良いか。人どうしと同じくらい人に近いか | `agree`（参照の作り方、評価者の配置、alternative annotator test） |
| 何件あれば足りるか | `power` |
| 本番でずれてきていないか | `trend`、`compare --unpaired` |

## 対象外

- AI のモデルを呼ぶことや、評価を走らせること。Inspect、promptfoo、自前のハーネスで走らせ、その記録を取り込んでください。
- プロンプトの管理、トレースの観測、ラベル付けの画面（v1）。
- 複数人で使うサーバーやクラウドのサービス。

## ライセンス

次のどちらかのライセンスで使えます。

- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT license（[LICENSE-MIT](LICENSE-MIT)）

どちらを選ぶかは利用者が決めます。

別に明示しない限り、Apache-2.0 ライセンスの定義に従ってこの成果物に含めるために意図して提出された貢献は、追加の条件なしに上の 2 つのライセンスで提供されるものとします。
