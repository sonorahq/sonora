equalizer-advanced = Lanjutan
equalizer-simple = Sederhana
equalizer-sound = Suara
equalizer-profile-default = Default
equalizer-profile-new = Profil { $number }
equalizer-profile-pasted = Profil tempelan
equalizer-profile-name-hint = Nama profil
equalizer-save-as = Simpan sebagai profil
equalizer-rename = Ganti nama profil
equalizer-delete = Hapus profil
equalizer-bass = Bas
equalizer-treble = Treble
equalizer-tone-reset = Reset ke 0 dB
equalizer-preamp = Preamp
equalizer-preamp-auto = Otomatis
equalizer-preamp-manual = Manual
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = okt
equalizer-no-gain = —
equalizer-filters = Filter
equalizer-no-filters = Belum ada filter. Tambahkan satu, pilih preset di atas, atau impor profil.
equalizer-add-filter = Tambah filter
equalizer-remove-filter = Hapus filter
equalizer-filter = Filter { $number }
equalizer-filter-kind = Jenis filter { $number }
equalizer-kind-peak = Peak
equalizer-kind-low-shelf = Low shelf
equalizer-kind-high-shelf = High shelf
equalizer-kind-low-pass = Low pass
equalizer-kind-high-pass = High pass
equalizer-kind-notch = Notch
equalizer-kind-band-pass = Band pass
equalizer-kind-all-pass = All pass
equalizer-transfer = Impor dan ekspor
equalizer-import = Impor
equalizer-export = Ekspor
equalizer-import-file = Impor file…
equalizer-export-file = Ekspor file…
equalizer-imported = { $name } diimpor
equalizer-imported-skipped = { $name } diimpor tanpa { $count ->
        [one] satu baris
       *[other] { $count } baris
    }
equalizer-import-nothing = Tidak ada pengaturan ekualiser yang bisa diimpor
equalizer-import-too-many = Tidak diimpor: { $count ->
        [one] ada satu filter
       *[other] ada { $count } filter
    }, melebihi kapasitas profil
equalizer-import-too-large = Tidak diimpor: teks lebih dari 1 MB, terlalu besar untuk profil ekualiser
equalizer-filter-limit = Satu profil dapat memuat hingga { $max } filter.
equalizer-paste-empty = Papan klip tidak berisi teks
equalizer-copied = Profil disalin sebagai teks Equalizer APO
equalizer-exported = Profil diekspor
equalizer-file-unreadable = File tidak dapat dibaca
equalizer-file-unwritable = File tidak dapat ditulis
equalizer-value-refused = Nilai sebelumnya dipertahankan
equalizer-skipped-line = Baris { $line }: { $reason }
equalizer-skipped-more = dan { $count ->
        [one] satu lagi
       *[other] { $count } lagi
    }
equalizer-skip-unknown-type = jenis filter { $name } tidak dikenal
equalizer-skip-unreadable = bukan filter yang dapat dibaca Sonora
equalizer-skip-no-frequency = tanpa frekuensi
equalizer-skip-no-gain = tanpa gain
equalizer-skip-no-width = tanpa Q atau bandwidth
equalizer-skip-expression = nilai ditulis sebagai ekspresi
equalizer-skip-no-preamp = preamp bukan angka
equalizer-skip-unsupported = { $name } tidak didukung
equalizer-rules = Aturan perangkat
equalizer-rule-other = Perangkat lainnya
equalizer-rule-playing = Sedang memutar
equalizer-rule-replaces = Menggantikan aturannya
equalizer-rule-choose-device = Pilih perangkat
equalizer-mode = Mode
equalizer-preamp-level = Tingkat preamp
equalizer-no-gain-spoken = Tanpa penguatan
equalizer-filter-frequency = Frekuensi filter { $number }
equalizer-filter-gain = Filter { $number } gain
equalizer-filter-width = Filter { $number } lebar
equalizer-graph = Respons frekuensi
equalizer-graph-detail = { $count ->
    [0] Tidak ada filter
    [one] satu filter
   *[other] { $count } filter
    }, preamp pada { $db } dB
equalizer-rule-device = Perangkat
equalizer-rule-choose-sound = Pilih suara
equalizer-rule-add = Tambah aturan
equalizer-rule-remove = Hapus aturan
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Frekuensi harus antara { $low } dan { $high } Hz
equalizer-invalid-gain = Gain harus antara { $low } dan { $high } dB
equalizer-invalid-q = Q harus antara { $low } dan { $high }
equalizer-invalid-octaves = Bandwidth harus antara { $low } dan { $high } oktaf
equalizer-invalid-shelf-octaves = Filter shelf memakai Q, bukan bandwidth
equalizer-invalid-preamp = Preamp harus antara { $low } dan { $high } dB
equalizer-overload = Clipping sebesar { $db } dB
equalizer-overload-slight = Clipping di bawah 0,1 dB
equalizer-lower-preamp = Turunkan preamp
equalizer-level = Level
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect dari Sonora
equalizer-status-server = Server suara lalu mengonversinya ke { $khz } kHz
equalizer-status-altered = Tidak bit-perfect karena:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volume
equalizer-status-normalized = Dinormalisasi
equalizer-status-direct = Dimainkan oleh Soloist, melewati Sonora
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } ch
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } ch
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
