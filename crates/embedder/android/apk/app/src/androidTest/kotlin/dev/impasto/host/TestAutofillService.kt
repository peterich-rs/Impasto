package dev.impasto.host

import android.os.Build
import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest
import androidx.test.filters.SdkSuppress

/** Enables the virtual-field path on test devices without a configured provider. */
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.O)
class TestAutofillService : AutofillService() {
    override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
        callback.onSuccess(null)
    }

    override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
        callback.onSuccess()
    }
}
