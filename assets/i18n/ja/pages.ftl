# login
login-restoring = 保存されたセッションを確認しています…
login-authorizing = ブラウザでの承認を待っています…
login-signed-in = { $name } としてサインイン中
login-failed-title = サインインに失敗しました
login-problem-region = 現在いる国からは Spotify のセッションを開始できません。居住国からサインインするか、Spotify アカウントの国設定を変更してください。
login-problem-credentials = 保存された Spotify のセッションは無効になっています。続けるにはもう一度サインインしてください。
login-problem-network = 音楽サービスに接続できませんでした。インターネット接続を確認してもう一度お試しください。
login-problem-cancelled = サインインを承認する前にブラウザのページが閉じられました。最初からやり直してください。
login-problem-refused = Spotify がサインインを拒否しました。しばらく待ってからもう一度お試しください。
login-problem-premium = Sonora は Spotify Premium でストリーミングしますが、このアカウントは Premium プランに加入していません。続けるには Premium アカウントでサインインしてください。
login-sign-in = { $provider } でサインイン
login-connect-cookies = Cookie を手動で貼り付け
login-cookie-open = { $provider } を開く
login-cookie-submit = 続行
login-cookie-hint = Cookie リクエストヘッダーをここに貼り付け
login-cookie-step-1 = music.youtube.com を開き、サインインしていることを確認します。シークレット ウィンドウが最適です。
login-cookie-step-2 = F12 を押して Network タブを開き、ページを再読み込みします。
login-cookie-step-3 = 「browse」または「next」という名前のリクエストを選択します。
login-cookie-step-4 = Headers の Request Headers にある Cookie を右クリックし、その値をコピーします。
login-cookie-step-note = 値全体を貼り付けてください。リクエストの Cookies パネルでは不十分で、値に SAPISID と __Secure-3PAPISID が含まれている必要があります。
login-cookie-header-title = サインインを完了するには { $provider } の Cookie を貼り付けてください
login-cookie-apple-step-3 = { $site } へのリクエストをどれか選択します。
login-cookie-apple-note = { $cookie } を含め、値全体を貼り付けてください。
login-cookie-named-title = サインインを完了するには { $provider } の Cookie を貼り付けてください
login-cookie-named-hint = Cookie の値をここに貼り付け
login-cookie-named-step-1 = { $site } を開き、サインインしていることを確認します。
login-cookie-named-step-2 = F12 を押し、Firefox では Storage タブ、Chrome では Application タブを開きます。
login-cookie-named-step-3 = Cookies を展開して { $site } を選び、{ $cookie } という名前の Cookie を探します。
login-cookie-named-step-4 = その値をダブルクリックしてコピーします。
login-cookie-named-note = { $cookie } の値だけで十分です。Network タブからコピーした Cookie ヘッダー全体でも構いません。
login-guest-title = ゲストモード
login-guest-use = ゲストモードを使う
login-usage-consent = Sonora の利用者数の推計にご協力ください。
login-device-code = { $url } でこのコードを入力してください
login-server-title = Subsonic サーバーに接続
login-server-detail = Subsonic または OpenSubsonic サーバー（Navidrome、Airsonic、Gonic など）のアドレスを入力し、サーバーのユーザー名とパスワードでサインインしてください。セッションはこのデバイスに保存されます。
login-server-hint = https://music.example.com
login-username-hint = ユーザー名
login-password-hint = パスワード
login-server-submit = 接続
login-account-title = アカウントを選択
login-account-detail = このセッションは複数の Google アカウントにサインインしています。Sonora が使うアカウントを選んでください。
login-choose-title = { $provider } にサインイン
login-choose-detail = { $provider } へのサインイン方法を選んでください。

# album and playlist pages

# artist page
artist-eyebrow = アーティスト
artist-monthly-listeners = { $count ->
   *[other] 月間リスナー { $value }人
}
artist-popular = 人気の曲
artist-popular-eyebrow = このアーティストを知る
artist-popular-empty = このアーティストにはまだ再生できる曲がありません
artist-releases = リリース
artist-filter-all = すべて
artist-filter-singles = シングル
artist-filter-eps = EP
artist-appears-on = 参加作品
album-also-like = おすすめ
album-tab-artists = アーティスト
album-label = ℗ { $label }

# user profile page
user-eyebrow = プロフィール
user-followers = { $count ->
   *[other] フォロワー { $value }人
}
user-following = { $count ->
   *[other] フォロー中 { $value }人
}
user-playlists = 公開プレイリスト
user-playlists-empty = 公開プレイリストはまだありません

# home page
home-quick-picks = クイックピック
home-quick-picks-empty = 何曲かお気に入りに追加するとここに表示されます
home-recently-added = 最近追加した項目
home-recently-played = 最近再生した項目
home-favorite-albums = お気に入りのアルバム
home-artists = アーティスト
home-collection-albums = コレクションのアルバム

# search page
search-placeholder = 何を聴きたいですか？
search-browse = すべてのジャンル
search-submit = 検索
search-recent = 最近の検索
search-forget = 履歴から削除
genre-empty = ここにはまだ何もありません
search-best-match = ベストマッチ
search-no-matches = 一致する項目がありません
search-results = 検索結果
search-artists = アーティスト
search-albums-playlists = アルバムとプレイリスト
search-tag = { $kind } ·
search-saved =
    { $count ->
       *[other] ライブラリに { $count }曲
    }

# song page
song-eyebrow = 曲
song-view-album = アルバムを表示
song-about = この曲について
song-album = アルバム
song-released = リリース日
song-streams = 再生回数
song-position = 順位
song-label = レーベル
song-popularity = 人気度
song-disc-track = ディスク { $disc }、トラック { $track }
song-track = トラック { $track }
song-credits = クレジット
song-performed-by = 演奏者
song-details = ジャンルと詳細
song-genres = ジャンル
song-language = 言語
song-content = コンテンツ
song-explicit = 露骨な表現あり
song-clean = 露骨な表現なし
artist-about = アーティストについて
artist-about-fallback = アーティストの人気曲やリリースを見てみましょう。
artist-about-open = アーティストを表示
song-copyright = © { $notice }

# tags
tags-edit-title = タグを編集
tags-sheet-song = 曲
tags-sheet-album = アルバム
tags-sheet-details = 詳細
tags-title = タイトル
tags-artist = アーティスト
tags-artists = アーティスト
tags-add-artist = アーティストを追加
tags-remove-artist = このアーティストを削除
tags-track = トラック番号
tags-track-total = リリースの曲数
tags-disc = ディスク番号
tags-disc-total = リリースのディスク数
tags-album = アルバム
tags-album-artist = アルバムアーティスト
tags-year = 年
tags-genre = ジャンル
tags-composer = 作曲者
tags-publisher = 発行元
tags-isrc = ISRC
tags-comment = コメント
