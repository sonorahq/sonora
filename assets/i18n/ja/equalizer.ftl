equalizer-advanced = 詳細
equalizer-simple = シンプル
equalizer-sound = サウンド
equalizer-profile-default = デフォルト
equalizer-profile-new = プロファイル { $number }
equalizer-profile-pasted = 貼り付けたプロファイル
equalizer-profile-name-hint = プロファイル名
equalizer-save-as = プロファイルとして保存
equalizer-rename = プロファイル名を変更
equalizer-delete = プロファイルを削除
equalizer-bass = 低音
equalizer-treble = 高音
equalizer-tone-reset = 0 dB にリセット
equalizer-preamp = プリアンプ
equalizer-preamp-auto = 自動
equalizer-preamp-manual = 手動
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct
equalizer-no-gain = —
equalizer-filters = フィルター
equalizer-no-filters = フィルターはまだありません。追加するか、上のプリセットを選ぶか、プロファイルをインポートしてください。
equalizer-add-filter = フィルターを追加
equalizer-remove-filter = フィルターを削除
equalizer-kind-peak = ピーク
equalizer-kind-low-shelf = ローシェルフ
equalizer-kind-high-shelf = ハイシェルフ
equalizer-kind-low-pass = ローパス
equalizer-kind-high-pass = ハイパス
equalizer-kind-notch = ノッチ
equalizer-kind-band-pass = バンドパス
equalizer-kind-all-pass = オールパス
equalizer-transfer = インポートとエクスポート
equalizer-import = インポート
equalizer-export = エクスポート
equalizer-import-file = ファイルをインポート…
equalizer-export-file = ファイルにエクスポート…
equalizer-imported = { $name } をインポートしました
equalizer-imported-skipped = { $name } をインポートしました（{ $count ->
       *[other] { $count }行
    }を除く）
equalizer-import-nothing = インポートできるイコライザー設定が見つかりませんでした
equalizer-import-too-many = インポートできません: 有効なフィルターが{ $count ->
       *[other] { $count }個
    }あり、プロファイルの上限を超えています
equalizer-import-too-large = インポートできません: テキストが 1 MB を超えており、イコライザープロファイルには大きすぎます
equalizer-filter-limit = プロファイルに入れられるフィルターは { $max } 個までです。
equalizer-paste-empty = クリップボードにテキストがありません
equalizer-copied = プロファイルを Equalizer APO 形式でコピーしました
equalizer-exported = プロファイルをエクスポートしました
equalizer-file-unreadable = ファイルを読み込めませんでした
equalizer-file-unwritable = ファイルを書き込めませんでした
equalizer-value-refused = 以前の値を維持しました
equalizer-skipped-line = { $line } 行目: { $reason }
equalizer-skipped-more = ほか{ $count ->
       *[other] { $count }件
    }
equalizer-skip-unknown-type = 不明なフィルタータイプ { $name }
equalizer-skip-unreadable = Sonora が読み込めるフィルターではありません
equalizer-skip-no-frequency = 周波数がありません
equalizer-skip-no-gain = ゲインがありません
equalizer-skip-no-width = Q または帯域幅がありません
equalizer-skip-expression = 式で書かれた値です
equalizer-skip-no-preamp = プリアンプが数値ではありません
equalizer-skip-unsupported = { $name } はサポートされていません
equalizer-rules = デバイスごとのルール
equalizer-rule-other = その他のデバイス
equalizer-rule-playing = 再生中
equalizer-rule-replaces = 既存のルールを置き換えます
equalizer-rule-choose-device = デバイスを選択
equalizer-rule-choose-sound = サウンドを選択
equalizer-rule-add = ルールを追加
equalizer-rule-remove = ルールを削除
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = 周波数は { $low }～{ $high } Hz の範囲で指定してください
equalizer-invalid-gain = ゲインは { $low }～{ $high } dB の範囲で指定してください
equalizer-invalid-q = Q は { $low }～{ $high } の範囲で指定してください
equalizer-invalid-octaves = 帯域幅は { $low }～{ $high } オクターブの範囲で指定してください
equalizer-invalid-shelf-octaves = シェルフには帯域幅ではなく Q を指定します
equalizer-invalid-preamp = プリアンプは { $low }～{ $high } dB の範囲で指定してください
equalizer-overload = { $db } dB のクリッピング
equalizer-overload-slight = 0.1 dB 未満のクリッピング
equalizer-lower-preamp = プリアンプを下げる
equalizer-level = レベル
equalizer-status-bit-perfect = ビットパーフェクト
equalizer-status-bit-perfect-detail = Sonora からはビットパーフェクト
equalizer-status-server = その後サウンドサーバーが { $khz } kHz に変換します
equalizer-status-altered = ビットパーフェクトでない理由:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = 音量
equalizer-status-normalized = 平準化
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to }ビット
equalizer-status-float = 浮動小数点 → { $to }ビット
equalizer-status-channels = { $from ->
    [1] モノラル
    [2] ステレオ
   *[other] { $from }ch
} → { $to ->
    [1] モノラル
    [2] ステレオ
   *[other] { $to }ch
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
