from flask import Flask, jsonify, request, send_from_directory
# import subprocess
# import json
# from google.protobuf import json_format
# import node_pb2
# import primitives_pb2
# import requests 

app = Flask(__name__, static_folder='/var/www//stable-channels/html', static_url_path='')

@app.route('/', defaults={'filename': 'index.html'})
@app.route('/<path:filename>')
def serve_static(filename):
    return send_from_directory(app.static_folder, filename)


@app.route('/balance')
def get_balance():
    return jsonify({"balance": 100.00, "price":27432.12})

# @app.route('/keysend', methods=['POST'])
# def keysend():
#     json_data = request.json
#     print(json_data)

#     if 'destination' not in json_data or 'amount_msat' not in json_data:
#         return jsonify(success=False, error="Missing necessary fields"), 400  # Bad Request

#     if not isinstance(json_data['amount_msat'], int):
#         return jsonify(success=False, error="amount_msat must be an integer"), 400

#     destination = json_data['destination']
#     amount_msat = json_data['amount_msat']

#     # Construct the command as a list
#     cmd = ["glcli", "keysend", destination, str(amount_msat)]
    
#     working_dir = "/home/ubuntu/greenlight"

#     try:
#         # Use subprocess.run, providing the command and capturing any output
#         result = subprocess.run(cmd, check=True, text=True, capture_output=True, cwd=working_dir)
#         # Log the output for debugging purposes
#         print("STDOUT:", result.stdout)
#         print("STDERR:", result.stderr)
#     except subprocess.CalledProcessError as e:
#         print("Error occurred:", str(e))
#         return jsonify(success=False, error="Failed to send keys via command line"), 500

#     return jsonify(success=True), 200

if __name__ == '__main__':
    app.run(debug=True, host='0.0.0.0', port=8080)
