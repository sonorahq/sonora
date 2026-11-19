members-title = Anggota
members-empty = Mulai atau bergabung dengan Sesi untuk melihat siapa saja yang sedang mendengarkan bersama Anda

# login
login-restoring = Memeriksa sesi yang tersimpan…
login-authorizing = Menunggu otorisasi di peramban…
login-signed-in = Masuk sebagai { $name }
login-failed-title = Gagal masuk
login-problem-region = Spotify tidak mendukung sesi dari wilayah tempat Anda berada saat ini. Masuk dari negara asal Anda, atau ubah pengaturan negara di akun Spotify Anda.
login-problem-credentials = Sesi Spotify yang tersimpan sudah tidak berlaku. Masuk kembali untuk melanjutkan.
login-problem-password = Server tidak menerima nama pengguna dan kata sandi tersebut. Periksa kembali dan coba lagi.
login-problem-network = Sonora tidak dapat terhubung ke layanan musik. Periksa koneksi internet Anda dan coba lagi.
login-problem-cancelled = Halaman peramban ditutup sebelum otorisasi selesai. Silakan coba lagi.
login-problem-refused = Spotify menolak permintaan masuk. Tunggu beberapa saat dan coba lagi.
login-problem-premium = Sonora memerlukan akun Spotify Premium untuk streaming. Akun ini tidak memiliki langganan Premium. Masuk dengan akun Premium untuk melanjutkan.
login-problem-subscription = Tidak ada langganan aktif pada Apple ID ini.
login-subscription-check = Periksa Apple Music
login-sign-in = Masuk dengan { $provider }
login-connect-cookies = Tempel cookie manual
login-cookie-open = Buka { $provider }
login-cookie-submit = Lanjutkan
login-cookie-hint = Tempel header Cookie permintaan di sini
login-cookie-step-1 = Buka music.youtube.com dan pastikan Anda sudah masuk.
login-cookie-step-2 = Tekan F12, buka tab Network, lalu muat ulang halaman.
login-cookie-step-3 = Pilih permintaan apa pun yang bernama "browse" atau "next".
login-cookie-step-4 = Di bagian Headers, cari Cookie di bawah Request Headers, klik kanan, lalu salin nilainya.
login-cookie-step-note = Pastikan untuk menyalin seluruh isinya, termasuk SAPISID dan __Secure-3PAPISID.
login-cookie-header-title = Tempel cookie { $provider } Anda untuk menyelesaikan proses masuk
login-cookie-apple-step-3 = Pilih permintaan apa pun ke { $site }.
login-cookie-apple-note = Pastikan untuk menempel seluruh nilainya, termasuk { $cookie }.
login-cookie-named-title = Tempel cookie { $provider } Anda untuk menyelesaikan proses masuk
login-cookie-named-hint = Tempel nilai cookie di sini
login-cookie-named-step-1 = Buka { $site } dan pastikan Anda sudah masuk.
login-cookie-named-step-2 = Tekan F12, lalu buka tab Storage di Firefox atau tab Application di Chrome.
login-cookie-named-step-3 = Buka Cookies, pilih { $site }, lalu cari cookie bernama { $cookie }.
login-cookie-named-step-4 = Klik dua kali nilainya, lalu salin.
login-cookie-named-note = Nilai { $cookie } saja sudah cukup. Seluruh header Cookie yang disalin dari tab Network juga bisa digunakan.
login-guest-title = Mode Tamu
login-guest-use = Gunakan Mode Tamu
login-usage-consent = Bantu kami memperkirakan jumlah pengguna aktif Sonora.
login-device-code = Masukkan kode ini di { $url }
login-server-title = Hubungkan ke server Subsonic Anda
login-server-detail = Masukkan alamat server Subsonic atau OpenSubsonic (Navidrome, Airsonic, Gonic, …), lalu masuk dengan nama pengguna dan kata sandi server Anda. Sesi hanya disimpan di perangkat ini.
login-server-hint = https://music.contoh.com
login-jellyfin-title = Hubungkan ke Jellyfin
login-jellyfin-detail = Masuk dengan akun Jellyfin Anda.
login-quick-connect = Masuk dengan Quick Connect
login-username-hint = Nama pengguna
login-password-hint = Kata sandi
login-server-submit = Hubungkan
login-account-title = Pilih akun
login-account-detail = Sesi ini terhubung ke lebih dari satu akun Google. Pilih akun yang ingin digunakan Sonora.
login-profile-title = Pilih profil
login-profile-detail = Akun ini memiliki lebih dari satu profil. Pilih profil yang akan digunakan Sonora.
login-choose-title = Masuk ke { $provider }
login-choose-detail = Pilih cara untuk Masuk ke { $provider }.
login-providers = Layanan musik
login-welcome-title = Selamat datang di Sonora
login-welcome-detail = Anda dapat menambahkan layanan lain nanti di Pengaturan.
login-or = atau

# album and playlist pages

# artist page
artist-eyebrow = Artis
artist-monthly-listeners = { $count ->
   *[other] { $value } pendengar bulanan
}
artist-followers = { $count ->
   *[other] { $value } pengikut
}
artist-popular = Populer
artist-popular-eyebrow = Jelajahi karya artis ini
artist-popular-empty = Belum ada lagu yang tersedia dari artis ini
artist-favorite-songs = Lagu-lagu favorit Anda
artist-favorite-songs-count = { $count ->
   *[other] { $count } lagu di Favorit Anda
}
artist-releases = Rilisan
artist-filter-all = Semua
artist-filter-singles = Single
artist-filter-eps = EP
artist-appears-on = Muncul di
album-also-like = Anda mungkin juga suka
album-tab-artists = Artis
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profil
user-followers = { $count ->
   *[other] { $value } pengikut
}
user-following = { $count ->
   *[other] { $value } mengikuti
}
user-playlists = Playlist publik
user-playlists-empty = Belum ada playlist publik

# home page
home-quick-picks = Pilihan cepat
home-quick-picks-empty = Sukai beberapa lagu dan rekomendasinya akan muncul di sini
home-recently-added = Baru ditambahkan
home-recently-played = Baru diputar
home-most-played = Paling sering diputar
home-favorite-albums = Album favorit
home-artists = Artis
home-collection-albums = Album dari koleksi Anda

# powerbar (quick search)
powerbar-title = Pencarian cepat
powerbar-open = Buka
powerbar-song = Buka lagu

# search page
search-placeholder = Apa yang ingin Anda dengarkan?
search-browse = Jelajahi semua
search-submit = Cari
search-open-link = Buka tautan di Sonora
search-recent = Pencarian terbaru
search-forget = Hapus dari riwayat
genre-empty = Belum ada yang bisa ditampilkan di sini
search-best-match = Hasil teratas
search-no-matches = Tidak ada hasil yang cocok
search-results = Hasil pencarian
search-artists = Artis
search-albums-playlists = Album & playlist
search-tag = { $kind } ·
search-saved = { $count ->
   *[other] { $count } lagu di Koleksi
}

# song page
song-eyebrow = Lagu
song-view-album = Lihat album
song-about = Tentang lagu ini
song-album = Album
song-released = Dirilis
song-streams = Stream
song-position = Posisi
song-label = Label
song-popularity = Popularitas
song-disc-track = Disk { $disc }, trek { $track }
song-track = Trek { $track }
song-credits = Kredit
song-performed-by = Dibawakan oleh
song-details = Genre & detail
song-genres = Genre
song-language = Bahasa
song-content = Konten
song-explicit = Eksplisit
song-local = File lokal
song-clean = Versi Bersih
artist-about = Tentang artis
artist-about-fallback = Jelajahi lagu populer dan rilisan dari artis ini.
artist-about-open = Buka profil artis
song-copyright = © { $notice }
track-row-name = { $title } oleh { $artists }

# tags
tags-edit-title = Edit tag
tags-sheet-song = Lagu
tags-sheet-album = Album
tags-sheet-details = Detail
tags-title = Judul
tags-artist = Artis
tags-artists = Artis
tags-add-artist = Tambah artis
tags-remove-artist = Hapus artis ini
tags-track = Nomor trek
tags-track-total = Jumlah trek dalam rilisan
tags-disc = Nomor disk
tags-disc-total = Jumlah disk dalam rilisan
tags-album = Album
tags-album-artist = Artis album
tags-year = Tahun
tags-genre = Genre
tags-composer = Komposer
tags-publisher = Penerbit
tags-isrc = ISRC
tags-comment = Komentar
