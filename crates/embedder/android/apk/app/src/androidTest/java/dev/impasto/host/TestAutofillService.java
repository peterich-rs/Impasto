package dev.impasto.host;

import android.os.CancellationSignal;
import android.service.autofill.AutofillService;
import android.service.autofill.FillCallback;
import android.service.autofill.FillRequest;
import android.service.autofill.SaveCallback;
import android.service.autofill.SaveRequest;

/** Enables virtual autofill fields from the test APK's standalone service process. */
public final class TestAutofillService extends AutofillService {
    @Override
    public void onFillRequest(FillRequest request, CancellationSignal signal, FillCallback callback) {
        callback.onSuccess(null);
    }

    @Override
    public void onSaveRequest(SaveRequest request, SaveCallback callback) {
        callback.onSuccess();
    }
}
