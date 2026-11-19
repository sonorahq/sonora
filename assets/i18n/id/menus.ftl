# track menu
menu-new-playlist = Playlist baru
menu-import-playlist = Impor playlist…
menu-no-playlists = Tidak ada playlist
menu-search-playlists = Cari playlist
menu-save-library = Perpustakaan
menu-add-to-library = Tambahkan ke Favorit
menu-remove-from-library = Hapus dari Favorit
menu-remove-tracks-from-playlist = { $count ->
   *[other] Hapus { $count } lagu dari playlist
}
menu-remove-tracks-from-history = { $count ->
   *[other] Hapus { $count } lagu dari riwayat
}
menu-download = Unduh
menu-remove-download = Hapus unduhan
menu-downloading = Mengunduh. Klik untuk berhenti
menu-song-radio = Buka radio lagu
menu-go-to-album = Buka album
menu-go-to-artist = Buka artis
menu-go-to-playlist = Buka daftar putar
menu-view-details = Lihat detail
menu-copy-link = Salin tautan
menu-select-all = Pilih semua
menu-remove-from-queue = Hapus dari antrean
menu-export-playlist = Ekspor playlist…
menu-add-playlist-to-library = Simpan ke Koleksi
menu-remove-playlist-from-library = Hapus dari Koleksi
menu-make-playlist-public = Jadikan publik
menu-make-playlist-private = Jadikan privat
menu-play-artist = Putar artis

# playlist editor
menu-copy-link-none = Item ini tidak memiliki tautan untuk dibagikan
menu-copy-link-local = Artis dan playlist lokal tidak memiliki tautan
menu-play-next = { $count ->
    [1] Putar berikutnya
   *[other] Putar { $count } lagu berikutnya
}
menu-add-to-queue = { $count ->
    [1] Tambahkan ke antrean
   *[other] Tambahkan { $count } lagu ke antrean
}
menu-play-last = { $count ->
    [1] Putar terakhir
   *[other] Putar { $count } lagu terakhir
}
menu-delete-track-files = { $count ->
    [1] Hapus file lagu
   *[other] Hapus { $count } file lagu
}
menu-add-to-playlist = { $count ->
    [1] Tambahkan ke daftar putar
   *[other] Tambahkan { $count } trek ke daftar putar
}

# smart rules
smart-rules-create-title = Daftar putar pintar baru
smart-rules-edit-title = Aturan pintar
smart-rules-loading = Membaca aturan dari server…
smart-rules-saving = Menyimpan…
smart-rules-failed-title = Aturan tidak dapat dibaca
smart-rules-match-all = Cocokkan semua
smart-rules-match-any = Cocokkan salah satu
smart-rules-match-toggle = Cocokkan semua aturan ini, atau salah satunya
smart-rules-of-following = dari yang berikut ini
smart-rules-add-condition = Tambahkan kondisi
smart-rules-add-group = Tambahkan grup
smart-rules-remove-group = Hapus grup ini
smart-rules-remove-condition = Hapus aturan ini
smart-rules-empty-group = Belum ada aturan di sini
smart-rules-field = Bidang
smart-rules-operator = Operator
smart-rules-value-placeholder = Nilai
smart-rules-range-and = dan
smart-rules-true = Ya
smart-rules-false = Tidak
smart-rules-unknown-note = Tetap seperti yang dikirim server
smart-rules-synced-title = Daftar putar ini disinkronkan dari sebuah file
smart-rules-synced-message = Pemindaian berikutnya dapat menimpa pengeditan yang dilakukan di sini.
smart-rules-sort-label = Urutkan berdasarkan
smart-rules-sort-placeholder = judul, tanggal disukai, acak…
smart-rules-order-none = Tanpa arah
smart-rules-order-asc = Menanjak
smart-rules-order-desc = Menurun
smart-rules-order-toggle = Arah penyortiran
smart-rules-limit-label = Batas
smart-rules-limit-placeholder = Semua yang cocok
smart-rules-problem-empty-group = Grup ini tidak berisi aturan apa pun
smart-rules-problem-missing-field = Kondisi ini tidak menyebutkan bidang apa pun
smart-rules-problem-range = Rentang memerlukan batas bawah dan batas atas
smart-rules-problem-days = Jumlah hari memerlukan angka
smart-rules-problem-playlist = Aturan daftar putar memerlukan daftar putar
smart-rules-problem-truth = Aturan memerlukan nilai kebenaran
smart-rules-problem-compared = Nilai tidak sesuai dengan aturan
smart-rules-problem-boolean = Bidang boolean memerlukan nilai kebenaran
smart-rules-problem-bad-order = Urutannya adalah asc atau desc

# playlist editor
playlist-name-placeholder = Nama playlist
playlist-create-title = Buat playlist
playlist-create-shelf = Simpan ke
playlist-edit-title = Edit playlist
playlist-cover-choose = Pilih gambar
playlist-cover-remove = Hapus sampul
playlist-cover-unopenable = Sonora tidak bisa membuka file ini
playlist-cover-too-large = Gambar ini lebih besar dari 40 MB
playlist-cover-not-a-picture = File ini bukan gambar JPEG, PNG, atau WebP

# cover viewer
playlist-delete-title = Hapus playlist
playlist-again-title = Tambahkan lagi?
playlist-again-confirm = Lagu ini sudah ada di “{ $name }”. Tetap tambahkan?
playlist-again-add = Tetap tambahkan

# confirm

# cover viewer
cover-view = Lihat sampul
cover-zoom-in = Perbesar
cover-zoom-out = Perkecil
cover-zoom-fit = Sesuaikan dengan jendela

# confirm
confirm-remove-library-title = Hapus dari koleksi
confirm-remove-playlist-title = Hapus dari playlist
confirm-remove-history-title = Hapus dari riwayat
confirm-remove-songs = { $count ->
    [one] Hapus lagu ini dari koleksi Anda?
   *[other] Hapus { $count } lagu dari koleksi Anda?
}
confirm-remove-playlist-songs = { $count ->
    [one] Hapus lagu ini dari playlist?
   *[other] Hapus { $count } lagu dari playlist?
}
confirm-remove-history-songs = { $count ->
    [one] Hapus lagu ini dari riwayat pemutaran?
   *[other] Hapus { $count } lagu dari riwayat pemutaran?
}
confirm-remove-albums = { $count ->
    [one] Hapus album ini dari koleksi Anda?
   *[other] Hapus { $count } album dari koleksi Anda?
}
confirm-remove-artists = { $count ->
    [one] Hapus artis ini dari Favorit?
   *[other] Hapus { $count } artis dari Favorit?
}
confirm-remove-playlists = { $count ->
    [one] Hapus playlist ini dari koleksi Anda?
   *[other] Hapus { $count } playlist dari koleksi Anda?
}
confirm-delete-track-files-title = Hapus file lagu?
confirm-delete-track-files = File akan dihapus permanen dari disk. Tindakan ini tidak dapat dibatalkan.
confirm-uninstall-widevine-title = Copot modul Widevine?
confirm-uninstall-widevine = Sonora akan menghapus salinan yang diunduhnya dari Google. Lagu Apple Music tidak dapat diputar sampai modul diunduh lagi.
confirm-remove-soloist-title = Hapus Soloist?
confirm-remove-soloist = Sonora akan menghapus program Soloist, kunci API Anda, dan data masuknya. Spotify akan kembali ke pemutar bawaan.
confirm-remove-downloads-title = Hapus semua unduhan?
confirm-remove-downloads = Semua lagu yang Anda unduh dari semua layanan akan dihapus dari komputer ini. Lagu-lagu itu akan di-stream lagi sampai Anda mengunduhnya kembali.
confirm-delete-named = Hapus “{ $name }”? Tindakan ini tidak dapat dibatalkan.
