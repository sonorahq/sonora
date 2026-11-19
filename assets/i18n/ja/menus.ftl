# track menu
menu-new-playlist = 新しいプレイリスト
menu-import-playlist = プレイリストをインポート…
menu-no-playlists = プレイリストがありません
menu-search-playlists = プレイリストを検索
menu-add-to-library = お気に入りに追加
menu-remove-from-library = お気に入りから削除
menu-remove-tracks-from-playlist = { $count }曲をプレイリストから削除
menu-remove-tracks-from-history = { $count }曲を履歴から削除
menu-download = ダウンロード
menu-remove-download = ダウンロードを削除
menu-downloading = ダウンロード中。クリックで停止
menu-song-radio = 曲のラジオを開く
menu-go-to-album = アルバムを表示
menu-go-to-artist = アーティストを表示
menu-view-details = 詳細を表示
menu-copy-link = リンクをコピー
menu-select-all = すべて選択
menu-remove-from-queue = 再生キューから削除
menu-export-playlist = プレイリストをエクスポート…
menu-add-playlist-to-library = ライブラリに追加
menu-remove-playlist-from-library = ライブラリから削除
menu-make-playlist-public = 公開する
menu-make-playlist-private = 非公開にする
menu-play-artist = アーティストを再生

# playlist editor
menu-copy-link-none = 共有できるリンクがありません
menu-copy-link-local = ローカルのアーティストとプレイリストにはリンクがありません
menu-play-next = { $count ->
    [1] 次に再生
   *[other] { $count }曲を次に再生
}
menu-add-to-queue = { $count ->
    [1] 再生キューに追加
   *[other] { $count }曲を再生キューに追加
}
menu-play-last = { $count ->
    [1] 最後に再生
   *[other] { $count }曲を最後に再生
}
menu-delete-track-files = { $count ->
    [1] 曲ファイルを削除
   *[other] { $count }個の曲ファイルを削除
}

# playlist editor
playlist-name-placeholder = プレイリスト名
playlist-create-title = プレイリストを作成
playlist-edit-title = プレイリストを編集
playlist-cover-choose = 画像を選択
playlist-cover-remove = カバーを削除
playlist-cover-unopenable = このファイルを開けません
playlist-cover-too-large = この画像は 40 MB を超えています
playlist-cover-not-a-picture = このファイルは JPEG、PNG、WebP の画像ではありません
playlist-delete-title = プレイリストを削除
playlist-again-title = もう一度追加しますか？
playlist-again-confirm = この曲は既に「{ $name }」にあります。もう一度追加しますか？
playlist-again-add = 再度追加

# confirm

# confirm
confirm-remove-library-title = ライブラリから削除
confirm-remove-playlist-title = プレイリストから削除
confirm-remove-history-title = 履歴から削除
confirm-remove-songs = { $count }曲をライブラリから削除しますか？
confirm-remove-playlist-songs = { $count }曲をプレイリストから削除しますか？
confirm-remove-history-songs = { $count }曲を再生履歴から削除しますか？
confirm-remove-albums = { $count }枚のアルバムをライブラリから削除しますか？
confirm-remove-artists = { $count ->
   *[other] { $count }人のアーティストをお気に入りから削除しますか？
}
confirm-remove-playlists = { $count }件のプレイリストをライブラリから削除しますか？
confirm-delete-track-files-title = 曲ファイルを削除しますか？
confirm-delete-track-files = ファイルはディスクから完全に削除されます。この操作は取り消せません。
confirm-uninstall-widevine-title = Widevine モジュールをアンインストールしますか？
confirm-uninstall-widevine = Google からダウンロードしたコピーを削除します。再度ダウンロードするまで Apple Music の曲は再生できません。
confirm-remove-downloads-title = すべてのダウンロードを削除しますか？
confirm-remove-downloads = ダウンロードした曲は、どのサービスのものもすべてこのコンピューターから削除されます。もう一度ダウンロードするまではストリーミングで再生されます。
confirm-delete-named = 「{ $name }」を削除しますか？この操作は取り消せません。
