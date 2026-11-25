equalizer-advanced = Nâng cao
equalizer-simple = Đơn giản
equalizer-sound = Âm thanh
equalizer-profile-default = Mặc định
equalizer-profile-new = Hồ sơ { $number }
equalizer-profile-pasted = Hồ sơ đã dán
equalizer-profile-name-hint = Tên hồ sơ
equalizer-save-as = Lưu thành hồ sơ
equalizer-rename = Đổi tên hồ sơ
equalizer-delete = Xóa hồ sơ
equalizer-bass = Âm trầm
equalizer-treble = Âm cao
equalizer-tone-reset = Đặt lại về 0 dB
equalizer-preamp = Tiền khuếch đại
equalizer-preamp-auto = Tự động
equalizer-preamp-manual = Thủ công
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct
equalizer-no-gain = —
equalizer-filters = Bộ lọc
equalizer-no-filters = Chưa có bộ lọc nào. Hãy thêm bộ lọc, chọn một cài đặt sẵn ở trên hoặc nhập một cấu hình.
equalizer-add-filter = Thêm bộ lọc
equalizer-remove-filter = Xóa bộ lọc
equalizer-filter = Bộ lọc { $number }
equalizer-filter-kind = Loại bộ lọc { $number }
equalizer-kind-peak = Đỉnh
equalizer-kind-low-shelf = Shelf thấp
equalizer-kind-high-shelf = Shelf cao
equalizer-kind-low-pass = Thông thấp
equalizer-kind-high-pass = Thông cao
equalizer-kind-notch = Chặn hẹp
equalizer-kind-band-pass = Thông dải
equalizer-kind-all-pass = Thông toàn dải
equalizer-transfer = Nhập và xuất
equalizer-import = Nhập
equalizer-export = Xuất
equalizer-import-file = Nhập tệp…
equalizer-export-file = Xuất tệp…
equalizer-imported = Đã nhập { $name }
equalizer-imported-skipped = Đã nhập { $name }, bỏ qua { $count ->
       *[other] { $count } dòng
    }
equalizer-import-nothing = Không tìm thấy cài đặt bộ chỉnh âm nào để nhập
equalizer-import-too-many = Chưa nhập: { $count ->
       *[other] { $count } bộ lọc đang bật
    }, nhiều hơn mức một cấu hình có thể chứa
equalizer-import-too-large = Chưa nhập: văn bản lớn hơn 1 MB, quá lớn đối với một cấu hình bộ chỉnh âm
equalizer-filter-limit =Một cấu hình chứa tối đa { $max } bộ lọc.
equalizer-paste-empty = Bộ nhớ tạm không có văn bản
equalizer-copied = Đã sao chép cấu hình dưới dạng văn bản Equalizer APO
equalizer-exported = Đã xuất cấu hình
equalizer-file-unreadable = Không thể đọc tệp
equalizer-file-unwritable = Không thể ghi tệp
equalizer-value-refused = Đã giữ giá trị trước đó
equalizer-skipped-line = Dòng { $line }: { $reason }
equalizer-skipped-more = và { $count ->
       *[other] { $count } dòng khác
    }
equalizer-skip-unknown-type = loại bộ lọc không xác định { $name }
equalizer-skip-unreadable = không phải bộ lọc mà Sonora đọc được
equalizer-skip-no-frequency = không có tần số
equalizer-skip-no-gain = không có mức tăng
equalizer-skip-no-width = không có Q hoặc băng thông
equalizer-skip-expression = giá trị được viết dưới dạng biểu thức
equalizer-skip-no-preamp = tiền khuếch đại không phải là số
equalizer-skip-unsupported = { $name } không được hỗ trợ
equalizer-rules = Quy tắc thiết bị
equalizer-rule-other = Mọi thiết bị khác
equalizer-rule-playing = Đang phát
equalizer-rule-replaces = Thay cho quy tắc hiện có
equalizer-rule-choose-device = Chọn thiết bị
equalizer-mode = Chế độ
equalizer-preamp-level = Mức tiền khuếch đại
equalizer-no-gain-spoken = Không có mức tăng
equalizer-filter-frequency = Tần số bộ lọc { $number }
equalizer-filter-gain = Mức tăng bộ lọc { $number }
equalizer-filter-width = Độ rộng bộ lọc { $number }
equalizer-graph = Đáp ứng tần số
equalizer-graph-detail = { $count ->
    [0] Không có bộ lọc
   *[other] { $count } bộ lọc
    }, tiền khuếch đại ở { $db } dB
equalizer-rule-device = Thiết bị
equalizer-rule-choose-sound = Chọn âm thanh
equalizer-rule-add = Thêm quy tắc
equalizer-rule-remove = Xóa quy tắc
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Tần số phải nằm trong khoảng { $low } đến { $high } Hz
equalizer-invalid-gain = Mức tăng phải nằm trong khoảng { $low } đến { $high } dB
equalizer-invalid-q = Q phải nằm trong khoảng { $low } đến { $high }
equalizer-invalid-octaves = Băng thông phải nằm trong khoảng { $low } đến { $high } quãng tám
equalizer-invalid-shelf-octaves = Bộ lọc shelf dùng Q, không dùng băng thông
equalizer-invalid-preamp = Tiền khuếch đại phải nằm trong khoảng { $low } đến { $high } dB
equalizer-overload = Cắt đỉnh { $db } dB
equalizer-overload-slight = Cắt đỉnh dưới 0,1 dB
equalizer-lower-preamp = Giảm tiền khuếch đại
equalizer-level = Mức
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect từ Sonora
equalizer-status-server = Sau đó máy chủ âm thanh chuyển đổi thành { $khz } kHz
equalizer-status-altered = Không bit-perfect do:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Âm lượng
equalizer-status-normalized = Đã chuẩn hóa
equalizer-status-direct = Phát qua Soloist, bỏ qua Sonora
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } kênh
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } kênh
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
